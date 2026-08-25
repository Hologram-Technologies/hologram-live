use crate::app::AppState;
use crate::auth::Principal;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{
    operation, ApplicationCompletion, HoloInspection, HoloPlan, HoloRunResult, OperationKind,
    ResidentHolo,
};
use axum::extract::{Extension, Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
        id: operation::HOLO_PLAN,
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
    OperationDescriptor {
        id: operation::HOLO_LOAD,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HOLO_UNLOAD,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HOLO_RUN,
        kind: OperationKind::Stream,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HOLO_RESIDENT,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
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
            .route("/api/v1/holo/{kappa}/plan", get(plan_holo))
            .route("/api/v1/holo/resident", get(resident_holo))
            .route(
                "/api/v1/holo/{kappa}/load",
                post(load_holo).delete(unload_holo),
            )
            .route("/api/v1/holo/{kappa}/run", post(run_holo))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <HoloApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_holo, inspect_holo, plan_holo, resident_holo, load_holo, unload_holo, run_holo),
    components(schemas(HoloInspection, HoloPlan, ResidentHolo, HoloRunResult, ApplicationCompletion, HoloRunHttpRequest)),
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

#[utoipa::path(
    get,
    path = "/api/v1/holo/{kappa}/plan",
    params(("kappa" = String, Path)),
    responses((status = 200, body = HoloPlan))
)]
pub async fn plan_holo(
    State(state): State<AppState>,
    Path(kappa): Path<String>,
) -> Result<Json<HoloPlan>, HttpError> {
    let catalog = state.holo_catalog().clone();
    let plan = tokio::task::spawn_blocking(move || catalog.plan(&kappa))
        .await
        .map_err(|error| crate::error::LiveError::Conflict(format!("join .holo plan: {error}")))??;
    Ok(Json(plan))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct HoloRunHttpRequest {
    #[serde(default)]
    pub inputs: Vec<Vec<u8>>,
}

#[utoipa::path(
    get,
    path = "/api/v1/holo/resident",
    responses((status = 200, body = [ResidentHolo]))
)]
pub async fn resident_holo(
    State(state): State<AppState>,
) -> Result<Json<Vec<ResidentHolo>>, HttpError> {
    Ok(Json(state.holo_runtime().list().await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/holo/{kappa}/load",
    params(("kappa" = String, Path)),
    responses((status = 200, body = ResidentHolo))
)]
pub async fn load_holo(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(kappa): Path<String>,
) -> Result<Json<ResidentHolo>, HttpError> {
    Ok(Json(
        state.holo_runtime().load_for(&kappa, &principal.id).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/holo/{kappa}/load",
    params(("kappa" = String, Path)),
    responses((status = 204))
)]
pub async fn unload_holo(
    State(state): State<AppState>,
    Path(kappa): Path<String>,
) -> Result<axum::http::StatusCode, HttpError> {
    state.holo_runtime().unload(&kappa).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/holo/{kappa}/run",
    params(("kappa" = String, Path)),
    request_body = HoloRunHttpRequest,
    responses((status = 200, body = HoloRunResult))
)]
pub async fn run_holo(
    State(state): State<AppState>,
    Path(kappa): Path<String>,
    Json(request): Json<HoloRunHttpRequest>,
) -> Result<Json<HoloRunResult>, HttpError> {
    Ok(Json(
        state.holo_runtime().run(&kappa, request.inputs).await?,
    ))
}
