use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, Conversation, OperationKind};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::HISTORY_CREATE,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HISTORY_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::HISTORY_GET,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::HISTORY_APPEND,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::HISTORY_DELETE,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.history",
    name: "Conversation History",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.kappa-registry"],
    operations: OPERATIONS,
};

pub struct HistoryModule;

impl LiveModule for HistoryModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new().route("/api/v1/history", get(list_history))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/history",
    responses((status = 200, body = [Conversation]))
)]
pub async fn list_history(
    State(state): State<AppState>,
) -> Result<Json<Vec<Conversation>>, HttpError> {
    let history = state.history().clone();
    let conversations = tokio::task::spawn_blocking(move || history.list())
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join history listing: {error}"))
        })??;
    Ok(Json(conversations))
}
