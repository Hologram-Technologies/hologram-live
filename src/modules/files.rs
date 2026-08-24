use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::registry;
use crate::protocol::{operation, OperationKind};
use axum::routing::get;
use axum::Router;

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
        Router::new().route("/api/v1/files", get(registry::list_objects))
    }
}
