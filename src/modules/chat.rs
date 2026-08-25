use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, Conversation, OperationKind};
use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

const OPERATIONS: &[OperationDescriptor] = &[OperationDescriptor {
    id: operation::CHAT_SEND,
    kind: OperationKind::Mutation,
    fallback_safe_before_dispatch: false,
}];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.chat",
    name: "Chat",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.history"],
    operations: OPERATIONS,
};

pub struct ChatModule;

impl LiveModule for ChatModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new().route("/api/v1/chat/{id}", post(send_message))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <ChatApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatRequest {
    pub content: String,
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(send_message),
    components(schemas(ChatRequest, Conversation)),
    tags((name = "chat", description = "Conversation-backed chat modules"))
)]
struct ChatApiDoc;

#[utoipa::path(
    post,
    path = "/api/v1/chat/{id}",
    params(("id" = String, Path, description = "Conversation ID")),
    request_body = ChatRequest,
    responses((status = 200, body = Conversation))
)]
pub async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<Conversation>, HttpError> {
    let conversation = state.chat().send(&id, request.content).await?;
    Ok(Json(conversation))
}
