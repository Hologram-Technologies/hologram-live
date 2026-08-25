use crate::app::AppState;
use crate::models::ModelInfo;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, OperationKind};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::MODEL_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::MODEL_IMPORT,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::MODEL_REMOVE,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.inference",
    name: "Inference",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.kappa-registry"],
    operations: OPERATIONS,
};

pub struct InferenceModule;

impl LiveModule for InferenceModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/models", get(list_models))
            .route("/api/v1/models/{id}", axum::routing::delete(remove_model))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <InferenceApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_models, remove_model),
    components(schemas(ModelInfo)),
    tags((name = "models", description = "Imported inference models"))
)]
struct InferenceApiDoc;

#[utoipa::path(
    get,
    path = "/api/v1/models",
    responses((status = 200, body = [ModelInfo]))
)]
pub async fn list_models(
    State(state): State<AppState>,
) -> Result<Json<Vec<ModelInfo>>, HttpError> {
    let models = state.models().clone();
    let records = tokio::task::spawn_blocking(move || models.list())
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join model listing: {error}"))
        })??;
    Ok(Json(records))
}

#[utoipa::path(
    delete,
    path = "/api/v1/models/{id}",
    params(("id" = String, Path, description = "Model ID")),
    responses((status = 204), (status = 404))
)]
pub async fn remove_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let models = state.models().clone();
    tokio::task::spawn_blocking(move || models.remove(&id))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join model removal: {error}"))
        })??;
    Ok(StatusCode::NO_CONTENT)
}
