use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, Conversation, OperationKind};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

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
    OperationDescriptor {
        id: operation::HISTORY_ARCHIVE,
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
        Router::new()
            .route("/api/v1/history", get(list_history).post(create_history))
            .route(
                "/api/v1/history/{id}",
                get(get_history).delete(delete_history),
            )
            .route("/api/v1/history/{id}/messages", post(append_history))
            .route("/api/v1/history/{id}/archive", post(archive_history))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <HistoryApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        list_history,
        create_history,
        get_history,
        append_history,
        delete_history,
        archive_history
    ),
    components(schemas(Conversation, CreateConversationRequest, AppendMessageRequest)),
    tags((name = "history", description = "Conversation history"))
)]
struct HistoryApiDoc;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConversationRequest {
    pub title: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AppendMessageRequest {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListConversationsQuery {
    /// Archived conversations are omitted unless this is set.
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ArchiveConversationRequest {
    pub archived: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/history",
    params(("include_archived" = Option<bool>, Query, description = "Include archived conversations")),
    responses((status = 200, body = [Conversation]))
)]
pub async fn list_history(
    State(state): State<AppState>,
    Query(query): Query<ListConversationsQuery>,
) -> Result<Json<Vec<Conversation>>, HttpError> {
    let history = state.history().clone();
    let conversations = tokio::task::spawn_blocking(move || history.list(query.include_archived))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join history listing: {error}"))
        })??;
    Ok(Json(conversations))
}

#[utoipa::path(
    post,
    path = "/api/v1/history",
    request_body = CreateConversationRequest,
    responses((status = 201, body = Conversation))
)]
pub async fn create_history(
    State(state): State<AppState>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<(StatusCode, Json<Conversation>), HttpError> {
    let history = state.history().clone();
    let conversation = tokio::task::spawn_blocking(move || history.create(request.title))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join history creation: {error}"))
        })??;
    Ok((StatusCode::CREATED, Json(conversation)))
}

#[utoipa::path(
    get,
    path = "/api/v1/history/{id}",
    params(("id" = String, Path, description = "Conversation ID")),
    responses((status = 200, body = Conversation), (status = 404))
)]
pub async fn get_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Conversation>, HttpError> {
    let history = state.history().clone();
    let conversation = tokio::task::spawn_blocking(move || history.get(&id))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join history read: {error}"))
        })??;
    Ok(Json(conversation))
}

#[utoipa::path(
    post,
    path = "/api/v1/history/{id}/messages",
    params(("id" = String, Path, description = "Conversation ID")),
    request_body = AppendMessageRequest,
    responses((status = 200, body = Conversation), (status = 404))
)]
pub async fn append_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AppendMessageRequest>,
) -> Result<Json<Conversation>, HttpError> {
    let history = state.history().clone();
    let conversation = tokio::task::spawn_blocking(move || {
        history.append(&id, request.role, request.content)
    })
    .await
    .map_err(|error| {
        crate::error::LiveError::Conflict(format!("join history append: {error}"))
    })??;
    Ok(Json(conversation))
}

#[utoipa::path(
    delete,
    path = "/api/v1/history/{id}",
    params(("id" = String, Path, description = "Conversation ID")),
    responses((status = 204))
)]
pub async fn delete_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let history = state.history().clone();
    tokio::task::spawn_blocking(move || history.delete(&id))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join history deletion: {error}"))
        })??;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/history/{id}/archive",
    params(("id" = String, Path, description = "Conversation id")),
    request_body = ArchiveConversationRequest,
    responses((status = 200, body = Conversation))
)]
pub async fn archive_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ArchiveConversationRequest>,
) -> Result<Json<Conversation>, HttpError> {
    let history = state.history().clone();
    let conversation = tokio::task::spawn_blocking(move || {
        history.set_archived(&id, request.archived)
    })
    .await
    .map_err(|error| {
        crate::error::LiveError::Conflict(format!("join history archive: {error}"))
    })??;
    Ok(Json(conversation))
}
