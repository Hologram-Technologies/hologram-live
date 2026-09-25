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

/// What a cluster request is authorized against: the names this node answers
/// to, and the policy that decides who may participate.
///
/// Held separately from [`AppState`] so that [`authorize_proof`] — the decision
/// every cluster request passes through — can be tested directly.
/// `AppState::build` opens the object store, starts the actor system and
/// constructs the inference engine, so a unit test cannot stand one up.
struct ClusterAuthority<'a> {
    node_id: &'a str,
    endpoint: Option<&'a str>,
    admission: &'a dyn crate::cluster::admission::Admission,
}

const MISADDRESSED: &str = "refused a cluster request addressed to another node; check that the \
     caller's configured seed and this node's cluster.advertise_endpoint name the same origin";

/// Reports a request addressed to some other node, at a level that an
/// unauthenticated caller cannot use to flood the log.
///
/// A proof that verifies says nothing about *who* sent it: `verify_signature`
/// checks the signature against the node id the caller supplied in its own
/// header, and the cluster routes carry no transport-level authentication, so
/// anyone can mint a keypair and sign a request naming an arbitrary recipient.
/// Warning unconditionally would therefore let unauthenticated input drive
/// unbounded warn-level writes. The diagnostic that matters is a
/// misconfiguration *between two peers that already admit each other*, so that
/// is the case kept at `warn`; everything else drops to `debug`.
///
/// Membership is read through `admitted()` rather than `authorize()` because
/// `authorize()` pins a ticket-bearing identity, and a request about to be
/// refused must not mutate admission state.
fn log_misaddressed_request(authority: &ClusterAuthority<'_>, caller: &str, recipient: &str) {
    // Endpoints and node ids are not secret, and without them an operator whose
    // seed spelling disagrees with this node's advertised endpoint has nothing
    // to diagnose a silently unformed cluster with. The signature, the
    // preimage, the token and the ticket are never logged.
    let accepted_node_id = authority.node_id;
    let accepted_endpoint = authority.endpoint.unwrap_or("<none advertised>");
    if authority.admission.admitted().contains(caller) {
        tracing::warn!(
            %recipient,
            node = %caller,
            %accepted_node_id,
            %accepted_endpoint,
            "{MISADDRESSED}"
        );
    } else {
        tracing::debug!(
            %recipient,
            node = %caller,
            %accepted_node_id,
            %accepted_endpoint,
            "{MISADDRESSED}"
        );
    }
}

fn authorize_proof(
    authority: &ClusterAuthority<'_>,
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
    // rather than guessed. Required, with no default and no fallback: a
    // recipient this node inferred for itself would defeat the point. It stays
    // untrusted until the signature verifies over this exact value, which is
    // why that check comes first — rewriting the header to name this node
    // invalidates the signature it was minted with.
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
    if !recipient_names_self(&recipient, authority.node_id, authority.endpoint) {
        log_misaddressed_request(authority, &proof.node_id, &recipient);
        return Err(HttpError(crate::error::LiveError::Authentication(
            "cluster request is addressed to another node".to_owned(),
        )));
    }
    let ticket = headers
        .get(crate::cluster::proof::TICKET_HEADER)
        .and_then(|value| value.to_str().ok());
    match authority.admission.authorize(&proof.node_id, ticket) {
        crate::cluster::admission::Decision::Admit => Ok(proof.node_id),
        crate::cluster::admission::Decision::Deny(reason) => Err(HttpError(
            crate::error::LiveError::Authorization(format!("cluster admission denied: {reason}")),
        )),
    }
}

fn authorize_cluster_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Result<String, HttpError> {
    let own_node_id = state.identity().node_id();
    let authority = ClusterAuthority {
        node_id: &own_node_id,
        endpoint: state.config().cluster.advertise_endpoint.as_deref(),
        admission: state.admission().as_ref(),
    };
    authorize_proof(&authority, headers, method, path, query, body)
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
    use crate::cluster::admission::AllowlistAdmission;
    use crate::cluster::identity::{NodeIdentity, KEY_FILE};
    use crate::cluster::proof::sign_request;
    use crate::cluster::proof::verify_request;
    use crate::cluster::proof::{
        RequestProof, NODE_HEADER, RECIPIENT_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER,
    };
    use axum::http::HeaderMap;

    const ORIGIN: &str = "https://node.example:11435";
    const ROGUE: &str = "https://rogue.example:11435";
    const OBJECTS: &str = "/api/v1/cluster/objects";

    fn identity(label: &str) -> (tempfile::TempDir, NodeIdentity) {
        let directory = tempfile::tempdir().unwrap_or_else(|_| panic!("{label} state directory"));
        let identity = NodeIdentity::load_or_create(&directory.path().join(KEY_FILE))
            .unwrap_or_else(|_| panic!("{label} identity"));
        (directory, identity)
    }

    /// The four headers a signed cluster request carries. Built by hand so a
    /// test can remove or rewrite one, which is exactly what the client cannot
    /// do for itself.
    fn headers_for(proof: &RequestProof, recipient: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            (NODE_HEADER, proof.node_id.as_str()),
            (TIMESTAMP_HEADER, proof.timestamp.as_str()),
            (SIGNATURE_HEADER, proof.signature.as_str()),
            (RECIPIENT_HEADER, recipient),
        ] {
            headers.insert(name, value.parse().expect("header value"));
        }
        headers
    }

    fn authorize(
        own_node_id: &str,
        admission: &dyn crate::cluster::admission::Admission,
        headers: &HeaderMap,
    ) -> Result<String, super::HttpError> {
        super::authorize_proof(
            &super::ClusterAuthority {
                node_id: own_node_id,
                endpoint: Some(ORIGIN),
                admission,
            },
            headers,
            "GET",
            OBJECTS,
            None,
            b"",
        )
    }

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

    // Fix round 3, finding D: the recipient header is required outright, and it
    // is genuinely covered by the signature rather than merely compared. Both
    // properties previously held only by construction.
    #[test]
    fn the_recipient_header_is_required_and_covered_by_the_signature() {
        let (_caller_dir, caller) = identity("caller");
        let (_node_dir, node) = identity("node");
        let own_node_id = node.node_id();
        let admission = AllowlistAdmission::new(vec![caller.node_id()]);

        // Baseline: correctly addressed, from an admitted identity.
        let good = sign_request(&caller, ORIGIN, "GET", OBJECTS, None, b"");
        assert_eq!(
            authorize(&own_node_id, &admission, &headers_for(&good, ORIGIN)).unwrap_or_else(
                |error| panic!("a correctly addressed request from an admitted node: {}", error.0)
            ),
            caller.node_id(),
            "authorization returns the verified caller"
        );

        // Absent: no default, no fallback to this node's own id or endpoint.
        let mut absent = headers_for(&good, ORIGIN);
        absent.remove(RECIPIENT_HEADER);
        let error = authorize(&own_node_id, &admission, &absent)
            .expect_err("a request with no recipient header must be refused");
        let message = error.0.to_string();
        assert!(message.contains(RECIPIENT_HEADER), "{message}");

        // Rewritten to name this node, over a signature minted for the rogue
        // origin. The assertion on *which* check rejects it is the point: it
        // must fail on the signature, proving the header is inside the
        // preimage rather than merely compared against it afterwards.
        let relayed = sign_request(&caller, ROGUE, "GET", OBJECTS, None, b"");
        let error = authorize(&own_node_id, &admission, &headers_for(&relayed, ORIGIN))
            .expect_err("a rewritten recipient header must be refused");
        let message = error.0.to_string();
        assert!(
            message.contains("invalid cluster request proof"),
            "must fail on the signature, got {message}"
        );
        assert!(
            !message.contains("addressed to another node"),
            "must not reach the recipient comparison, got {message}"
        );

        // Left naming the rogue, the same proof verifies and is refused by the
        // recipient comparison instead — the other half of the pair.
        let error = authorize(&own_node_id, &admission, &headers_for(&relayed, ROGUE))
            .expect_err("a request addressed elsewhere must be refused");
        assert!(
            error.0.to_string().contains("addressed to another node"),
            "{}",
            error.0
        );
    }

    // Fix round 3, finding D: a proof can be perfectly valid and still belong to
    // nobody this node admits. Admission is the last gate, not the first.
    #[test]
    fn a_valid_proof_from_an_unadmitted_node_is_refused() {
        let (_caller_dir, caller) = identity("caller");
        let (_node_dir, node) = identity("node");
        let own_node_id = node.node_id();
        let proof = sign_request(&caller, ORIGIN, "GET", OBJECTS, None, b"");
        let headers = headers_for(&proof, ORIGIN);

        let closed = AllowlistAdmission::new(Vec::new());
        let error = authorize(&own_node_id, &closed, &headers)
            .expect_err("an unadmitted identity must be refused");
        assert!(
            matches!(error.0, crate::error::LiveError::Authorization(_)),
            "admission refusal is an authorization failure, not an authentication one"
        );
        assert!(
            error.0.to_string().contains("cluster admission denied"),
            "{}",
            error.0
        );

        // The very same request, once the identity is trusted: this is what
        // makes the refusal above attributable to admission and nothing else.
        let open = AllowlistAdmission::new(vec![caller.node_id()]);
        assert_eq!(
            authorize(&own_node_id, &open, &headers)
                .unwrap_or_else(|error| panic!("an admitted identity: {}", error.0)),
            caller.node_id()
        );
    }
}
