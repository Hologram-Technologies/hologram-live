use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, ObjectMetadata, OperationKind};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[OperationDescriptor {
    id: operation::REGISTRY_LIST,
    kind: OperationKind::Read,
    fallback_safe_before_dispatch: true,
}];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.kappa-registry",
    name: "Kappa Registry Provider",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.system"],
    operations: OPERATIONS,
};

pub struct KappaRegistryModule;

impl LiveModule for KappaRegistryModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new().route("/api/v1/objects", get(list_objects))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/objects",
    responses((status = 200, body = [ObjectMetadata]))
)]
pub async fn list_objects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ObjectMetadata>>, HttpError> {
    let store = state.store().clone();
    let objects = tokio::task::spawn_blocking(move || store.list(None))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join object listing: {error}"))
        })??;
    Ok(Json(objects))
}
