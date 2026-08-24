use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::protocol::{operation, CapabilityManifest, ModuleInfo, OperationKind};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::SYSTEM_HANDSHAKE,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::SYSTEM_HEALTH,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::SYSTEM_SHUTDOWN,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::MODULES_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::TRACING_GET,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::TRACING_SET,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.system",
    name: "Hologram Live System",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &[],
    operations: OPERATIONS,
};

pub struct SystemModule;

impl LiveModule for SystemModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/modules", get(list_modules))
            .route("/api/v1/capabilities", get(capabilities))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/modules",
    responses((status = 200, body = [ModuleInfo]))
)]
pub async fn list_modules(State(state): State<AppState>) -> Json<Vec<ModuleInfo>> {
    Json(state.module_info())
}

#[utoipa::path(
    get,
    path = "/api/v1/capabilities",
    responses((status = 200, body = CapabilityManifest))
)]
pub async fn capabilities(State(state): State<AppState>) -> Json<CapabilityManifest> {
    Json(state.capability_manifest())
}
