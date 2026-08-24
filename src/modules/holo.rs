use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, HoloInspection, OperationKind};
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::HOLO_IMPORT,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HOLO_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::HOLO_INSPECT,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::HOLO_VERIFY,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::HOLO_REMOVE,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.holo",
    name: "Hologram Applications",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.kappa-registry"],
    operations: OPERATIONS,
};

pub struct HoloModule;

impl LiveModule for HoloModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/holo", get(list_holo))
            .route("/api/v1/holo/{kappa}", get(inspect_holo))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <HoloApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_holo, inspect_holo),
    components(schemas(HoloInspection)),
    tags((name = "holo", description = "Hologram application archives"))
)]
struct HoloApiDoc;

#[utoipa::path(
    get,
    path = "/api/v1/holo",
    responses((status = 200, body = [HoloInspection]))
)]
pub async fn list_holo(
    State(state): State<AppState>,
) -> Result<Json<Vec<HoloInspection>>, HttpError> {
    let catalog = state.holo_catalog().clone();
    let records = tokio::task::spawn_blocking(move || catalog.list())
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join .holo listing: {error}"))
        })??;
    Ok(Json(records))
}

#[utoipa::path(
    get,
    path = "/api/v1/holo/{kappa}",
    params(("kappa" = String, Path)),
    responses((status = 200, body = HoloInspection))
)]
pub async fn inspect_holo(
    State(state): State<AppState>,
    Path(kappa): Path<String>,
) -> Result<Json<HoloInspection>, HttpError> {
    let catalog = state.holo_catalog().clone();
    let inspection = tokio::task::spawn_blocking(move || catalog.inspect(&kappa))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join .holo inspect: {error}"))
        })??;
    Ok(Json(inspection))
}
