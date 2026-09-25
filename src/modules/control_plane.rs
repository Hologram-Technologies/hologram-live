use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, NodeRecord, ObjectPage, ObjectQuery, OperationKind};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse};
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::NODES_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::NODES_HEARTBEAT,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.control-plane",
    name: "Control Plane Foundation",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.system"],
    operations: OPERATIONS,
};

pub struct ControlPlaneModule;

impl LiveModule for ControlPlaneModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/nodes", get(list_nodes))
            .route("/api/v1/nodes/owner", get(get_mutable_owner))
            .route("/api/v1/nodes/placement", get(get_capable_owner))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <ControlPlaneApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_nodes, get_mutable_owner, get_capable_owner, join_cluster),
    components(schemas(NodeRecord, ClusterJoinRequest, ClusterJoinResponse)),
    tags((name = "control-plane", description = "Node inventory"))
)]
struct ControlPlaneApiDoc;

#[utoipa::path(
    get,
    path = "/api/v1/nodes",
    responses((status = 200, body = [NodeRecord]))
)]
pub async fn list_nodes(State(state): State<AppState>) -> Result<Json<Vec<NodeRecord>>, HttpError> {
    let nodes = state.nodes().clone();
    let records = tokio::task::spawn_blocking(move || nodes.list())
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join node listing: {error}"))
        })??;
    Ok(Json(records))
}

#[derive(serde::Deserialize)]
pub struct OwnerQuery {
    resource: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/nodes/owner",
    params(("resource" = String, Query, description = "Stable mutable resource key")),
    responses((status = 200, body = NodeRecord), (status = 404, description = "No reachable owner"))
)]
pub async fn get_mutable_owner(
    State(state): State<AppState>,
    Query(query): Query<OwnerQuery>,
) -> Result<Json<NodeRecord>, HttpError> {
    if query.resource.is_empty() || query.resource.len() > 4_096 {
        return Err(HttpError(crate::error::LiveError::Protocol(
            "resource must be between 1 and 4096 bytes".to_owned(),
        )));
    }
    let owner = tokio::task::spawn_blocking(move || state.mutable_owner(&query.resource))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("select mutable owner: {error}"))
        })??
        .ok_or_else(|| {
            HttpError(crate::error::LiveError::NotFound(
                "no reachable cluster owner".to_owned(),
            ))
        })?;
    Ok(Json(owner))
}

#[derive(serde::Deserialize)]
pub struct PlacementQuery {
    resource: String,
    operation: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/nodes/placement",
    params(
        ("resource" = String, Query, description = "Stable placement key"),
        ("operation" = String, Query, description = "Required advertised operation")
    ),
    responses((status = 200, body = NodeRecord), (status = 404, description = "No capable reachable node"))
)]
pub async fn get_capable_owner(
    State(state): State<AppState>,
    Query(query): Query<PlacementQuery>,
) -> Result<Json<NodeRecord>, HttpError> {
    if query.resource.is_empty()
        || query.resource.len() > 4_096
        || query.operation.is_empty()
        || query.operation.len() > 256
    {
        return Err(HttpError(crate::error::LiveError::Protocol(
            "resource and operation must be non-empty and within their size bounds".to_owned(),
        )));
    }
    let owner = tokio::task::spawn_blocking(move || {
        state.capable_owner(&query.resource, &query.operation)
    })
    .await
    .map_err(|error| crate::error::LiveError::Conflict(format!("select placement: {error}")))??
    .ok_or_else(|| {
        HttpError(crate::error::LiveError::NotFound(
            "no reachable node advertises the required operation".to_owned(),
        ))
    })?;
    Ok(Json(owner))
}

#[utoipa::path(
    post,
    path = "/api/v1/cluster/join",
    request_body = ClusterJoinRequest,
    responses(
        (status = 200, body = ClusterJoinResponse),
        (status = 401, description = "Invalid cluster proof")
    )
)]
pub async fn join_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ClusterJoinResponse>, HttpError> {
    if body.len() > crate::cluster::MAX_JOIN_BYTES {
        return Err(HttpError(crate::error::LiveError::Protocol(format!(
            "cluster join request exceeds {} bytes",
            crate::cluster::MAX_JOIN_BYTES
        ))));
    }
    let config = &state.config().cluster;
    let advertised = config.advertise_endpoint.as_ref().ok_or_else(|| {
        crate::error::LiveError::NotFound("cluster membership is not enabled".to_owned())
    })?;
    // The join path is a fixed route, so the signed path is this constant
    // rather than a value read off the request: nothing a caller sends can
    // steer it.
    let signer = authorize_cluster_request(
        &state,
        &headers,
        "POST",
        crate::cluster::JOIN_PATH,
        None,
        &body,
    )?;
    let request: ClusterJoinRequest = serde_json::from_slice(&body)
        .map_err(crate::error::LiveError::from)
        .map_err(HttpError)?;
    crate::cluster::validate_node_record(&request.node).map_err(HttpError)?;
    record_matches_signer(&request.node, &signer)?;

    let nodes = state.nodes().clone();
    let joining = request.node;
    tokio::task::spawn_blocking(move || nodes.heartbeat(joining))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join cluster heartbeat: {error}"))
        })??;

    let local = state.local_node_record(advertised.trim_end_matches('/').to_owned());
    let nodes = state.nodes().clone();
    let local_heartbeat = local.clone();
    let peers = tokio::task::spawn_blocking(move || {
        nodes.heartbeat(local_heartbeat)?;
        nodes.list()
    })
    .await
    .map_err(|error| {
        crate::error::LiveError::Conflict(format!("join cluster response: {error}"))
    })??;
    Ok(Json(ClusterJoinResponse { node: local, peers }))
}

pub async fn list_cluster_objects(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    authorize_cluster_request(&state, &headers, "GET", uri.path(), uri.query(), &[])?;
    let registry = state.registry().clone();
    let objects = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| crate::error::LiveError::Conflict(format!("join cluster inventory: {error}")))??;
    Ok(Json(objects))
}

pub async fn get_cluster_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
) -> Result<axum::response::Response, HttpError> {
    authorize_cluster_request(&state, &headers, "GET", uri.path(), uri.query(), &[])?;
    let registry = state.registry().clone();
    let object = tokio::task::spawn_blocking(move || registry.get_object(&id))
        .await
        .map_err(|error| crate::error::LiveError::Conflict(format!("join cluster object read: {error}")))??;
    let metadata = object.metadata.clone();
    let mut response = crate::modules::registry::object_response(object)?;
    let headers = response.headers_mut();
    headers.insert(
        "x-hologram-object-kind",
        axum::http::HeaderValue::from_str(&metadata.kind)
            .map_err(|error| HttpError(crate::error::LiveError::Protocol(format!("cluster object kind header: {error}"))))?,
    );
    headers.insert(
        "x-hologram-object-created-at-millis",
        axum::http::HeaderValue::from_str(&metadata.created_at_millis.to_string())
            .map_err(|error| HttpError(crate::error::LiveError::Protocol(format!("cluster object timestamp header: {error}"))))?,
    );
    if let Some(filename) = metadata.filename {
        headers.insert(
            "x-hologram-object-filename",
            axum::http::HeaderValue::from_str(&filename)
                .map_err(|error| HttpError(crate::error::LiveError::Protocol(format!("cluster object filename header: {error}"))))?,
        );
    }
    Ok(response)
}

fn record_matches_signer(node: &crate::protocol::NodeRecord, signer: &str) -> Result<(), HttpError> {
    if node.node_id != signer {
        return Err(HttpError(crate::error::LiveError::Authentication(
            "cluster node record does not match the signing identity".to_owned(),
        )));
    }
    Ok(())
}

/// Whether the recipient a caller signed against names *this* node.
///
/// Two spellings name this node and both stay accepted. A peer that holds our
/// key addresses our `node_id`, compared by exact equality — Phase 2's iroh
/// addresses *are* node ids and have no origin, so that branch is load-bearing
/// for the next phase. A peer reaching us over HTTP addresses the origin it
/// dialled, which is the only name it can be sure of before it has learned our
/// key; that one is compared as a parsed origin, so a spelling difference in
/// host case, default port, or trailing slash does not silently refuse a
/// correctly configured peer forever.
///
/// What must never be accepted is a *third* spelling, and the corresponding
/// outbound rule is that `cluster::recipient_for` binds the origin being
/// dialled and never a node id learned from another peer's unsigned response.
/// Binding such an id would let an admitted-but-rogue peer name a victim,
/// collect the proof we mint, and replay it against the victim — which under
/// asymmetric `cluster.trusted_keys` reaches a node that refuses the rogue.
fn recipient_names_self(recipient: &str, own_node_id: &str, own_endpoint: Option<&str>) -> bool {
    recipient == own_node_id
        || own_endpoint.is_some_and(|endpoint| crate::cluster::same_origin(recipient, endpoint))
}

fn authorize_cluster_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Result<String, HttpError> {
    let proof = crate::cluster::proof::RequestProof {
        node_id: header(headers, crate::cluster::proof::NODE_HEADER)?.to_owned(),
        timestamp: header(headers, crate::cluster::proof::TIMESTAMP_HEADER)?.to_owned(),
        signature: header(headers, crate::cluster::proof::SIGNATURE_HEADER)?.to_owned(),
    };
    // The recipient the caller signed against, echoed so it can be compared
    // rather than guessed. Untrusted until the signature verifies over this
    // exact value, which is why the check below runs in this order: an
    // unauthenticated caller cannot reach the warning, and a caller that does
    // reach it has proved it holds a key, so the log line means a real
    // misconfiguration rather than noise.
    let recipient = header(headers, crate::cluster::proof::RECIPIENT_HEADER)?.to_owned();
    crate::cluster::proof::reject_separators(method, path, query, &recipient).map_err(HttpError)?;
    if let Err(error) =
        crate::cluster::proof::verify_request(&proof, &recipient, method, path, query, body)
    {
        tracing::debug!(%error, node = %proof.node_id, %method, %path, "cluster request proof did not verify");
        return Err(HttpError(crate::error::LiveError::Authentication(
            "invalid cluster request proof".to_owned(),
        )));
    }
    let own_node_id = state.identity().node_id();
    let own_endpoint = state.config().cluster.advertise_endpoint.as_deref();
    if !recipient_names_self(&recipient, &own_node_id, own_endpoint) {
        // Endpoints and node ids are not secret, and without them an operator
        // whose seed spelling disagrees with this node's advertised endpoint
        // has nothing to diagnose a silently unformed cluster with. The
        // signature, the preimage, the token and the ticket are never logged.
        tracing::warn!(
            %recipient,
            node = %proof.node_id,
            accepted_node_id = %own_node_id,
            accepted_endpoint = %own_endpoint.unwrap_or("<none advertised>"),
            "refused a cluster request addressed to another node; check that the caller's \
             configured seed and this node's cluster.advertise_endpoint name the same origin"
        );
        return Err(HttpError(crate::error::LiveError::Authentication(
            "cluster request is addressed to another node".to_owned(),
        )));
    }
    let ticket = headers
        .get(crate::cluster::proof::TICKET_HEADER)
        .and_then(|value| value.to_str().ok());
    match state.admission().authorize(&proof.node_id, ticket) {
        crate::cluster::admission::Decision::Admit => Ok(proof.node_id),
        crate::cluster::admission::Decision::Deny(reason) => Err(HttpError(
            crate::error::LiveError::Authorization(format!("cluster admission denied: {reason}")),
        )),
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, HttpError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            HttpError(crate::error::LiveError::Authentication(format!(
                "missing {name} header"
            )))
        })
}

#[cfg(test)]
mod tests {
    use crate::cluster::identity::{NodeIdentity, KEY_FILE};
    use crate::cluster::proof::sign_request;
    use crate::cluster::proof::verify_request;

    // Defect 4: a proof minted for one peer must not open another.
    #[test]
    fn a_proof_for_one_peer_is_refused_by_another() {
        let dir = tempfile::tempdir().expect("state directory");
        let caller = NodeIdentity::load_or_create(&dir.path().join(KEY_FILE)).expect("identity");
        let proof = sign_request(&caller, "ed25519:aa", "GET", "/api/v1/cluster/objects", None, b"");
        assert!(
            verify_request(&proof, "ed25519:bb", "GET", "/api/v1/cluster/objects", None, b"")
                .is_err()
        );
    }

    // Review Focus 3: signing correctly as A while claiming to be B proves nothing.
    #[test]
    fn a_record_that_disagrees_with_the_signer_is_refused() {
        let dir = tempfile::tempdir().expect("state directory");
        let signer = NodeIdentity::load_or_create(&dir.path().join(KEY_FILE)).expect("identity");
        let other = tempfile::tempdir().expect("other state directory");
        let claimed = NodeIdentity::load_or_create(&other.path().join(KEY_FILE)).expect("other");

        let mut record = crate::protocol::NodeRecord {
            node_id: claimed.node_id(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: "https://node.example".to_owned(),
            last_seen_millis: 0,
        };
        assert!(super::record_matches_signer(&record, &signer.node_id()).is_err());

        record.node_id = signer.node_id();
        assert!(super::record_matches_signer(&record, &signer.node_id()).is_ok());
    }

    // Fix round 1: an outbound proof binds the origin it was minted for, so an
    // admitted-but-rogue peer cannot relay a proof we minted for *its* origin
    // to a node that trusts us and refuses it. The relayed proof carries the
    // caller's real, legitimately admitted `node_id` and a signature that
    // verifies perfectly — neither the node header nor the signature is what
    // fails here, the recipient binding is.
    #[test]
    fn a_proof_minted_for_one_origin_is_refused_by_a_node_at_another() {
        const ROGUE_ORIGIN: &str = "https://rogue.example:11435";
        const VICTIM_ORIGIN: &str = "https://victim.example:11435";

        let caller_dir = tempfile::tempdir().expect("caller state directory");
        let caller =
            NodeIdentity::load_or_create(&caller_dir.path().join(KEY_FILE)).expect("identity");
        let victim_dir = tempfile::tempdir().expect("victim state directory");
        let victim =
            NodeIdentity::load_or_create(&victim_dir.path().join(KEY_FILE)).expect("victim");

        let relayed = sign_request(
            &caller,
            ROGUE_ORIGIN,
            "GET",
            "/api/v1/cluster/objects",
            None,
            b"",
        );
        assert_eq!(
            relayed.node_id,
            caller.node_id(),
            "the relayed proof must carry an identity the victim admits"
        );
        verify_request(
            &relayed,
            ROGUE_ORIGIN,
            "GET",
            "/api/v1/cluster/objects",
            None,
            b"",
        )
        .expect("the signature itself is valid: only the recipient disqualifies it");
        assert!(
            !super::recipient_names_self(ROGUE_ORIGIN, &victim.node_id(), Some(VICTIM_ORIGIN)),
            "a proof minted for another origin must not name this node"
        );
    }

    // Fix round 2, finding A: origin-only binding must not turn a spelling
    // difference into a cluster that silently never forms. Every spelling of
    // this node's own endpoint is accepted; a different host is not.
    #[test]
    fn a_differently_spelled_endpoint_still_names_this_node() {
        let dir = tempfile::tempdir().expect("state directory");
        let node = NodeIdentity::load_or_create(&dir.path().join(KEY_FILE)).expect("identity");
        let own_node_id = node.node_id();
        let advertised = Some("https://node.example");

        // The node id, compared by exact equality. Phase 2 signs only this form.
        assert!(super::recipient_names_self(
            &own_node_id,
            &own_node_id,
            advertised
        ));
        // Spellings of the advertised origin that must all be accepted.
        for recipient in [
            "https://node.example",
            "https://Node.Example",
            "https://node.example:443",
            "https://NODE.example:443/",
            "https://node.example/",
        ] {
            assert!(
                super::recipient_names_self(recipient, &own_node_id, advertised),
                "recipient {recipient} names this node"
            );
        }
        // A genuinely different origin must still be refused.
        for recipient in [
            "https://other.example",
            "https://node.example:11435",
            "http://node.example",
            "https://node.example.evil.test",
        ] {
            assert!(
                !super::recipient_names_self(recipient, &own_node_id, advertised),
                "recipient {recipient} must not name this node"
            );
        }
        // With nothing advertised, only the node id can name this node.
        assert!(super::recipient_names_self(&own_node_id, &own_node_id, None));
        assert!(!super::recipient_names_self(
            "https://node.example",
            &own_node_id,
            None
        ));
    }
}
