use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::{registry, HttpError};
use crate::protocol::{operation, ObjectMetadata, OperationKind};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[OperationDescriptor {
    id: operation::FILES_LIST,
    kind: OperationKind::Read,
    fallback_safe_before_dispatch: true,
}];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.files",
    name: "Artifact Files",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.kappa-registry"],
    operations: OPERATIONS,
};

pub struct FilesModule;

impl LiveModule for FilesModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new().route("/api/v1/files", get(list_files))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <FilesApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_files),
    components(schemas(ObjectMetadata)),
    tags((name = "files", description = "Artifact file discovery"))
)]
struct FilesApiDoc;

#[utoipa::path(
    get,
    path = "/api/v1/files",
    responses((status = 200, body = [ObjectMetadata]))
)]
pub async fn list_files(
    State(state): State<AppState>,
) -> Result<Json<Vec<ObjectMetadata>>, HttpError> {
    registry::list_objects(State(state)).await
}
