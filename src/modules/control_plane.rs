use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, NodeRecord, OperationKind};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
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
            .route(crate::cluster::JOIN_PATH, post(join_cluster))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <ControlPlaneApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_nodes, join_cluster),
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
