//! OpenAI-compatible HTTP API (non-streaming subset).
//!
//! A thin translation layer over the Phase-1 inference core: chat messages
//! render to the same `role: content` transcript the native chat module uses,
//! and completions come from the configured [`InferenceEngine`]. Token
//! streaming is not supported; `stream: true` is rejected with a typed 400 in
//! the `OpenAI` error envelope.

use crate::app::AppState;
use crate::error::LiveError;
use crate::inference::{CompletionRequest, InferenceEngine};
use crate::models::{ModelCatalog, ModelInfo};
use crate::module::{LiveModule, ModuleDescriptor};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.openai-compat",
    name: "OpenAI-Compatible API",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.inference"],
    operations: &[],
};

pub struct OpenAiCompatModule;

impl LiveModule for OpenAiCompatModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <OpenAiApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatCompletionRequest {
    /// Catalog model id or name; falls back to `inference.default_model`.
    #[serde(default)]
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatCompletion {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

/// The engine does not report token counts yet, so every field is null.
#[derive(Debug, Serialize, ToSchema)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<ModelObject>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelObject {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OpenAiErrorEnvelope {
    pub error: OpenAiErrorBody,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OpenAiErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub code: Option<String>,
}

/// Error rendered in the `OpenAI` envelope rather than the native `ApiError`.
#[derive(Debug)]
pub struct OpenAiError {
    status: StatusCode,
    body: OpenAiErrorBody,
}

impl OpenAiError {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: OpenAiErrorBody {
                message: message.into(),
                kind: "invalid_request_error".to_owned(),
                code: None,
            },
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: OpenAiErrorBody {
                message: message.into(),
                kind: "not_found_error".to_owned(),
                code: None,
            },
        }
    }

    fn server(error: &LiveError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: OpenAiErrorBody {
                message: error.to_string(),
                kind: "server_error".to_owned(),
                code: Some(error.code().to_owned()),
            },
        }
    }
}

impl From<LiveError> for OpenAiError {
    fn from(error: LiveError) -> Self {
        match error {
            LiveError::NotFound(_) => {
                let mut mapped = Self::not_found(error.to_string());
                mapped.body.code = Some(error.code().to_owned());
                mapped
            }
            LiveError::Config(_) | LiveError::Protocol(_) | LiveError::InvalidHolo(_) => {
                let mut mapped = Self::invalid_request(error.to_string());
                mapped.body.code = Some(error.code().to_owned());
                mapped
            }
            _ => Self::server(&error),
        }
    }
}

impl IntoResponse for OpenAiError {
    fn into_response(self) -> Response {
        (self.status, Json(OpenAiErrorEnvelope { error: self.body })).into_response()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(chat_completions, list_models),
    components(schemas(
        ChatCompletionRequest,
        ChatMessage,
        ChatCompletion,
        ChatChoice,
        Usage,
        ModelList,
        ModelObject,
        OpenAiErrorEnvelope,
        OpenAiErrorBody
    )),
    tags((name = "openai-compat", description = "OpenAI-compatible API (non-streaming)"))
)]
struct OpenAiApiDoc;

#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, body = ChatCompletion),
        (status = 400, body = OpenAiErrorEnvelope),
        (status = 404, body = OpenAiErrorEnvelope)
    )
)]
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletion>, OpenAiError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    Ok(Json(
        complete_chat(engine, catalog, &default_model, request).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/v1/models",
    responses((status = 200, body = ModelList))
)]
pub async fn list_models(State(state): State<AppState>) -> Result<Json<ModelList>, OpenAiError> {
    let catalog = state.models().clone();
    let models = tokio::task::spawn_blocking(move || catalog.list())
        .await
        .map_err(|error| {
            OpenAiError::server(&LiveError::Conflict(format!("join model listing: {error}")))
        })?
        .map_err(OpenAiError::from)?;
    Ok(Json(model_list(models)))
}

/// Core request mapping, kept free of `AppState` so unit tests can drive it
/// with a bare engine and catalog.
async fn complete_chat(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: ChatCompletionRequest,
) -> Result<ChatCompletion, OpenAiError> {
    if request.stream == Some(true) {
        return Err(OpenAiError::invalid_request(
            "streaming is not supported; omit stream or send stream: false",
        ));
    }
    if request.messages.is_empty() {
        return Err(OpenAiError::invalid_request("messages must not be empty"));
    }
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let completion = engine
        .complete(CompletionRequest {
            prompt: render_prompt(&request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            seed: request.seed,
            session_key: None,
        })
        .await
        .map_err(OpenAiError::from)?;
    let created = unix_seconds();
    Ok(ChatCompletion {
        id: completion_id(created, &completion.text),
        object: "chat.completion".to_owned(),
        created,
        model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_owned(),
                content: completion.text,
            },
            finish_reason: "stop".to_owned(),
        }],
        usage: Usage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
    })
}

/// Resolves the requested model to the label echoed in the response. The
/// echo engine accepts anything (it is the no-engine fallback); other engines
/// validate against the catalog, then against engine-reported models so
/// remote engine tags (e.g. Ollama's) remain usable.
async fn resolve_model(
    engine: &Arc<dyn InferenceEngine>,
    catalog: &Arc<ModelCatalog>,
    default_model: &str,
    requested: &str,
) -> Result<String, OpenAiError> {
    let name = if requested.trim().is_empty() {
        default_model.trim()
    } else {
        requested.trim()
    };
    if name.is_empty() {
        return Ok(engine.name().to_owned());
    }
    if engine.name() == "echo" {
        return Ok(name.to_owned());
    }
    let lookup = name.to_owned();
    let found = {
        let catalog = catalog.clone();
        let lookup = lookup.clone();
        tokio::task::spawn_blocking(move || catalog.resolve(&lookup))
            .await
            .map_err(|error| {
                OpenAiError::server(&LiveError::Conflict(format!("join model lookup: {error}")))
            })?
    };
    if let Ok(info) = found {
        return Ok(info.name);
    }
    let engine_models = engine.list_models().await.unwrap_or_default();
    if engine_models
        .iter()
        .any(|model| model.id == lookup || model.name == lookup)
    {
        Ok(lookup)
    } else {
        Err(OpenAiError::not_found(format!("model {name:?} not found")))
    }
}

fn model_list(models: Vec<ModelInfo>) -> ModelList {
    ModelList {
        object: "list".to_owned(),
        data: models
            .into_iter()
            .map(|model| ModelObject {
                id: model.id,
                object: "model".to_owned(),
                created: model.created_at_millis / 1000,
                owned_by: "hologram".to_owned(),
            })
            .collect(),
    }
}

/// Same `role: content` transcript shape as `chat::render_transcript`.
fn render_prompt(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|message| format!("{}: {}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn completion_id(created: u64, text: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&created.to_le_bytes());
    hasher.update(text.as_bytes());
    let hex = hasher.finalize().to_hex();
    format!("chatcmpl-{}", &hex[..24])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::Completion;

    struct Fixture {
        temporary: tempfile::TempDir,
        catalog: Arc<ModelCatalog>,
    }

    fn fixture() -> Fixture {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let store = Arc::new(
            crate::store::ObjectStore::open(temporary.path().join("store")).expect("store"),
        );
        let catalog =
            Arc::new(ModelCatalog::open(store, temporary.path().join("models")).expect("catalog"));
        Fixture { temporary, catalog }
    }

    fn import_model(fixture: &Fixture) -> ModelInfo {
        let artifact = fixture.temporary.path().join("tiny.wcpu");
        std::fs::create_dir_all(&artifact).expect("artifact dir");
        std::fs::write(artifact.join("manifest.json"), b"{}").expect("manifest");
        fixture.catalog.import(&artifact).expect("import")
    }

    /// Replies with the prompt verbatim so tests can assert the transcript.
    struct MirrorEngine;

    #[tonic::async_trait]
    impl InferenceEngine for MirrorEngine {
        fn name(&self) -> &'static str {
            "mirror"
        }

        async fn complete(&self, request: CompletionRequest) -> crate::error::Result<Completion> {
            Ok(Completion {
                text: request.prompt,
                model: "mirror".to_owned(),
                tokens_per_second: None,
                elapsed_millis: 0,
                usage: None,
            })
        }

        async fn list_models(&self) -> crate::error::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    fn request(model: &str, contents: &[&str]) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_owned(),
            messages: contents
                .iter()
                .map(|content| ChatMessage {
                    role: "user".to_owned(),
                    content: (*content).to_owned(),
                })
                .collect(),
            max_tokens: None,
            temperature: None,
            seed: None,
            stream: None,
        }
    }

    #[tokio::test]
    async fn echo_request_maps_to_a_valid_chat_completion() {
        let fixture = fixture();
        let completion = complete_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "",
            request("gpt-test", &["hello world"]),
        )
        .await
        .expect("completion");

        assert_eq!(completion.object, "chat.completion");
        assert!(completion.id.starts_with("chatcmpl-"));
        assert!(completion.created > 0);
        assert_eq!(completion.model, "gpt-test");
        assert_eq!(completion.choices.len(), 1);
        assert_eq!(completion.choices[0].index, 0);
        assert_eq!(completion.choices[0].finish_reason, "stop");
        assert_eq!(completion.choices[0].message.role, "assistant");
        assert_eq!(completion.choices[0].message.content, "hello world");
        assert_eq!(completion.usage.prompt_tokens, None);
        assert_eq!(completion.usage.completion_tokens, None);
        assert_eq!(completion.usage.total_tokens, None);
    }

    #[tokio::test]
    async fn messages_render_as_a_transcript_prompt() {
        let fixture = fixture();
        let mut conversation = request("", &[]);
        conversation.messages = vec![
            ChatMessage {
                role: "system".to_owned(),
                content: "be brief".to_owned(),
            },
            ChatMessage {
                role: "user".to_owned(),
                content: "ping".to_owned(),
            },
        ];
        let completion = complete_chat(
            Arc::new(MirrorEngine),
            fixture.catalog.clone(),
            "",
            conversation,
        )
        .await
        .expect("completion");

        assert_eq!(
            completion.choices[0].message.content,
            "system: be brief\nuser: ping"
        );
    }

    #[tokio::test]
    async fn stream_true_is_rejected_with_an_openai_error() {
        let fixture = fixture();
        let mut streamed = request("", &["hi"]);
        streamed.stream = Some(true);
        let error = complete_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "",
            streamed,
        )
        .await
        .expect_err("must fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.kind, "invalid_request_error");
        assert!(error.body.message.contains("streaming"));
    }

    #[tokio::test]
    async fn unknown_model_is_a_not_found_error() {
        let fixture = fixture();
        let error = complete_chat(
            Arc::new(MirrorEngine),
            fixture.catalog.clone(),
            "",
            request("ghost", &["hi"]),
        )
        .await
        .expect_err("must fail");

        assert_eq!(error.status, StatusCode::NOT_FOUND);
        assert_eq!(error.body.kind, "not_found_error");
    }

    #[tokio::test]
    async fn empty_model_falls_back_to_the_default_model() {
        let fixture = fixture();
        let imported = import_model(&fixture);
        let completion = complete_chat(
            Arc::new(MirrorEngine),
            fixture.catalog.clone(),
            &imported.id,
            request("", &["hi"]),
        )
        .await
        .expect("completion");

        assert_eq!(completion.model, "tiny");
    }

    #[tokio::test]
    async fn model_resolves_by_catalog_name() {
        let fixture = fixture();
        let imported = import_model(&fixture);
        let completion = complete_chat(
            Arc::new(MirrorEngine),
            fixture.catalog.clone(),
            "",
            request("tiny", &["hi"]),
        )
        .await
        .expect("completion");

        assert_eq!(completion.model, imported.name);
    }

    #[test]
    fn model_list_uses_the_openai_list_shape() {
        let fixture = fixture();
        let imported = import_model(&fixture);
        let list = model_list(fixture.catalog.list().expect("list"));

        assert_eq!(list.object, "list");
        assert_eq!(list.data.len(), 1);
        assert_eq!(list.data[0].id, imported.id);
        assert_eq!(list.data[0].object, "model");
        assert_eq!(list.data[0].owned_by, "hologram");
        assert_eq!(list.data[0].created, imported.created_at_millis / 1000);
    }

    #[test]
    fn error_envelope_serializes_in_the_openai_shape() {
        let response = OpenAiError::not_found("missing").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let envelope = OpenAiErrorEnvelope {
            error: OpenAiErrorBody {
                message: "missing".to_owned(),
                kind: "not_found_error".to_owned(),
                code: None,
            },
        };
        let json = serde_json::to_value(envelope).expect("serialize");
        assert_eq!(json["error"]["message"], "missing");
        assert_eq!(json["error"]["type"], "not_found_error");
        assert_eq!(json["error"]["code"], serde_json::Value::Null);
    }
}
