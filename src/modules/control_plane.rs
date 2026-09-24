use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, NodeRecord, ObjectPage, ObjectQuery, OperationKind};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
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
    let token = state.cluster_token().ok_or_else(|| {
        crate::error::LiveError::Authentication("cluster token is unavailable".to_owned())
    })?;
    let timestamp = header(&headers, crate::cluster::TIMESTAMP_HEADER)?;
    let signature = header(&headers, crate::cluster::SIGNATURE_HEADER)?;
    crate::cluster::verify(token, timestamp, signature, &body)?;
    let request: ClusterJoinRequest = serde_json::from_slice(&body)
        .map_err(crate::error::LiveError::from)
        .map_err(HttpError)?;
    crate::cluster::validate_node_record(&request.node).map_err(HttpError)?;

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
    Query(query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    verify_cluster_request(&state, &headers, &[])?;
    let registry = state.registry().clone();
    let objects = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| crate::error::LiveError::Conflict(format!("join cluster inventory: {error}")))??;
    Ok(Json(objects))
}

pub async fn get_cluster_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<axum::response::Response, HttpError> {
    verify_cluster_request(&state, &headers, &[])?;
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

fn verify_cluster_request(state: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<(), HttpError> {
    let token = state.cluster_token().ok_or_else(|| {
        HttpError(crate::error::LiveError::Authentication("cluster token is unavailable".to_owned()))
    })?;
    crate::cluster::verify(
        token,
        header(headers, crate::cluster::TIMESTAMP_HEADER)?,
        header(headers, crate::cluster::SIGNATURE_HEADER)?,
        body,
    )
    .map_err(HttpError)
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
