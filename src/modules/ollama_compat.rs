//! Ollama-compatible HTTP API (non-streaming subset).
//!
//! A thin translation layer over the Phase-1 inference core, mirroring
//! `openai_compat`. Token streaming is not supported; `stream: true` is
//! rejected with a 400 in Ollama's plain `{"error": "..."}` envelope.

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
    id: "dev.hologram.live.ollama-compat",
    name: "Ollama-Compatible API",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.inference"],
    operations: &[],
};

pub struct OllamaCompatModule;

impl LiveModule for OllamaCompatModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/generate", post(generate))
            .route("/api/chat", post(chat))
            .route("/api/tags", get(list_tags))
            .route("/api/show", post(show_model))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <OllamaApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GenerateRequest {
    /// Catalog model id or name; falls back to `inference.default_model`.
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub options: Option<OllamaOptions>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatRequest {
    /// Catalog model id or name; falls back to `inference.default_model`.
    #[serde(default)]
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub options: Option<OllamaOptions>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
pub struct OllamaOptions {
    #[serde(default)]
    pub num_predict: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GenerateResponse {
    pub model: String,
    pub created_at: String,
    pub response: String,
    pub done: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatResponse {
    pub model: String,
    pub created_at: String,
    pub message: OllamaMessage,
    pub done: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagsResponse {
    pub models: Vec<TagModel>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagModel {
    pub name: String,
    pub model: String,
    pub modified_at: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShowRequest {
    /// Ollama renamed this field from `name` to `model`; accept both.
    #[serde(default, alias = "name")]
    pub model: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShowResponse {
    pub modelfile: String,
    pub parameters: String,
    pub template: String,
    pub details: ModelDetails,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelDetails {
    pub format: String,
    pub family: String,
    pub families: Vec<String>,
    pub parameter_size: String,
    pub quantization_level: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OllamaErrorBody {
    pub error: String,
}

/// Error rendered in Ollama's plain `{"error": "..."}` envelope.
#[derive(Debug)]
pub struct OllamaError {
    status: StatusCode,
    message: String,
}

impl OllamaError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn server(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl From<LiveError> for OllamaError {
    fn from(error: LiveError) -> Self {
        match error {
            LiveError::NotFound(_) => Self::not_found(error.to_string()),
            LiveError::Config(_) | LiveError::Protocol(_) | LiveError::InvalidHolo(_) => {
                Self::bad_request(error.to_string())
            }
            other => Self::server(other.to_string()),
        }
    }
}

impl IntoResponse for OllamaError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(OllamaErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(generate, chat, list_tags, show_model),
    components(schemas(
        GenerateRequest,
        ChatRequest,
        OllamaOptions,
        OllamaMessage,
        GenerateResponse,
        ChatResponse,
        TagsResponse,
        TagModel,
        ShowRequest,
        ShowResponse,
        ModelDetails,
        OllamaErrorBody
    )),
    tags((name = "ollama-compat", description = "Ollama-compatible API (non-streaming)"))
)]
struct OllamaApiDoc;

#[utoipa::path(
    post,
    path = "/api/generate",
    request_body = GenerateRequest,
    responses(
        (status = 200, body = GenerateResponse),
        (status = 400, body = OllamaErrorBody),
        (status = 404, body = OllamaErrorBody)
    )
)]
pub async fn generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, OllamaError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    Ok(Json(
        generate_core(engine, catalog, &default_model, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/chat",
    request_body = ChatRequest,
    responses(
        (status = 200, body = ChatResponse),
        (status = 400, body = OllamaErrorBody),
        (status = 404, body = OllamaErrorBody)
    )
)]
pub async fn chat(
    State(state): State<AppState>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, OllamaError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    Ok(Json(
        chat_core(engine, catalog, &default_model, request).await?,
    ))
}

/// Core request mappings, kept free of `AppState` so unit tests can drive
/// them with a bare engine and catalog.
async fn generate_core(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: GenerateRequest,
) -> Result<GenerateResponse, OllamaError> {
    if request.stream == Some(true) {
        return Err(OllamaError::bad_request(
            "streaming is not supported; send stream: false",
        ));
    }
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let options = request.options.unwrap_or_default();
    let completion = engine
        .complete(CompletionRequest {
            prompt: request.prompt,
            max_tokens: options.num_predict,
            temperature: options.temperature,
            seed: options.seed,
        })
        .await
        .map_err(OllamaError::from)?;
    Ok(GenerateResponse {
        model,
        created_at: rfc3339_now(),
        response: completion.text,
        done: true,
    })
}

async fn chat_core(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: ChatRequest,
) -> Result<ChatResponse, OllamaError> {
    if request.stream == Some(true) {
        return Err(OllamaError::bad_request(
            "streaming is not supported; send stream: false",
        ));
    }
    if request.messages.is_empty() {
        return Err(OllamaError::bad_request("messages must not be empty"));
    }
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let options = request.options.unwrap_or_default();
    let completion = engine
        .complete(CompletionRequest {
            prompt: render_prompt(&request.messages),
            max_tokens: options.num_predict,
            temperature: options.temperature,
            seed: options.seed,
        })
        .await
        .map_err(OllamaError::from)?;
    Ok(ChatResponse {
        model,
        created_at: rfc3339_now(),
        message: OllamaMessage {
            role: "assistant".to_owned(),
            content: completion.text,
        },
        done: true,
    })
}

#[utoipa::path(
    get,
    path = "/api/tags",
    responses((status = 200, body = TagsResponse))
)]
pub async fn list_tags(State(state): State<AppState>) -> Result<Json<TagsResponse>, OllamaError> {
    let catalog = state.models().clone();
    let models = tokio::task::spawn_blocking(move || catalog.list())
        .await
        .map_err(|error| OllamaError::server(format!("join model listing: {error}")))?
        .map_err(OllamaError::from)?;
    Ok(Json(tags_from(models)))
}

#[utoipa::path(
    post,
    path = "/api/show",
    request_body = ShowRequest,
    responses(
        (status = 200, body = ShowResponse),
        (status = 400, body = OllamaErrorBody),
        (status = 404, body = OllamaErrorBody)
    )
)]
pub async fn show_model(
    State(state): State<AppState>,
    Json(request): Json<ShowRequest>,
) -> Result<Json<ShowResponse>, OllamaError> {
    if request.model.trim().is_empty() {
        return Err(OllamaError::bad_request("model is required"));
    }
    let catalog = state.models().clone();
    let name = request.model.trim().to_owned();
    let info = tokio::task::spawn_blocking(move || catalog.resolve(&name))
        .await
        .map_err(|error| OllamaError::server(format!("join model lookup: {error}")))?
        .map_err(OllamaError::from)?;
    Ok(Json(show_from(&info)))
}

/// Same resolution policy as the OpenAI-compat module: the echo engine
/// accepts any label, other engines validate against the catalog and then
/// against engine-reported models.
async fn resolve_model(
    engine: &Arc<dyn InferenceEngine>,
    catalog: &Arc<ModelCatalog>,
    default_model: &str,
    requested: &str,
) -> Result<String, OllamaError> {
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
            .map_err(|error| OllamaError::server(format!("join model lookup: {error}")))?
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
        Err(OllamaError::not_found(format!("model {name:?} not found")))
    }
}

fn tags_from(models: Vec<ModelInfo>) -> TagsResponse {
    TagsResponse {
        models: models
            .into_iter()
            .map(|model| TagModel {
                name: model.name.clone(),
                model: model.name,
                modified_at: rfc3339_from_millis(model.created_at_millis),
                size: model.size,
                digest: model.id,
            })
            .collect(),
    }
}

fn show_from(model: &ModelInfo) -> ShowResponse {
    ShowResponse {
        modelfile: String::new(),
        parameters: String::new(),
        template: String::new(),
        details: ModelDetails {
            format: "wcpu".to_owned(),
            family: model.engine.clone(),
            families: vec![model.engine.clone()],
            parameter_size: String::new(),
            quantization_level: String::new(),
        },
    }
}

/// Same `role: content` transcript shape as `chat::render_transcript`.
fn render_prompt(messages: &[OllamaMessage]) -> String {
    messages
        .iter()
        .map(|message| format!("{}: {}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rfc3339_now() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    rfc3339_from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
}

/// Formats milliseconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`
/// (Howard Hinnant's civil-from-days algorithm; no date crate dependency).
fn rfc3339_from_millis(millis: u64) -> String {
    let seconds = millis / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let secs_of_day = seconds % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z.rem_euclid(146_097)).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
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
            })
        }

        async fn list_models(&self) -> crate::error::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    fn generate_request(model: &str, prompt: &str) -> GenerateRequest {
        GenerateRequest {
            model: model.to_owned(),
            prompt: prompt.to_owned(),
            stream: Some(false),
            options: None,
        }
    }

    #[tokio::test]
    async fn generate_maps_to_the_ollama_response_shape() {
        let state_fixture = fixture();
        let response = generate_core(
            Arc::new(crate::inference::EchoEngine),
            state_fixture.catalog.clone(),
            "",
            generate_request("tiny", "hello"),
        )
        .await
        .expect("generate");

        assert_eq!(response.model, "tiny");
        assert_eq!(response.response, "hello");
        assert!(response.done);
        assert!(response.created_at.ends_with('Z'));
    }

    #[tokio::test]
    async fn chat_messages_render_as_a_transcript() {
        let state_fixture = fixture();
        let request = ChatRequest {
            model: String::new(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_owned(),
                    content: "be brief".to_owned(),
                },
                OllamaMessage {
                    role: "user".to_owned(),
                    content: "ping".to_owned(),
                },
            ],
            stream: None,
            options: None,
        };
        let response = chat_core(
            Arc::new(MirrorEngine),
            state_fixture.catalog.clone(),
            "",
            request,
        )
        .await
        .expect("chat");

        assert_eq!(response.message.role, "assistant");
        assert_eq!(response.message.content, "system: be brief\nuser: ping");
        assert!(response.done);
    }

    #[tokio::test]
    async fn stream_true_is_rejected() {
        let state_fixture = fixture();
        let mut request = generate_request("", "hello");
        request.stream = Some(true);
        let error = generate_core(
            Arc::new(crate::inference::EchoEngine),
            state_fixture.catalog.clone(),
            "",
            request,
        )
        .await
        .expect_err("must fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("streaming"));

        let chat_request = ChatRequest {
            model: String::new(),
            messages: vec![OllamaMessage {
                role: "user".to_owned(),
                content: "hi".to_owned(),
            }],
            stream: Some(true),
            options: None,
        };
        let error = chat_core(
            Arc::new(crate::inference::EchoEngine),
            state_fixture.catalog.clone(),
            "",
            chat_request,
        )
        .await
        .expect_err("must fail");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_model_is_a_not_found_error() {
        let state_fixture = fixture();
        let engine: Arc<dyn InferenceEngine> = Arc::new(MirrorEngine);
        let error = resolve_model(&engine, &state_fixture.catalog, "", "ghost")
            .await
            .expect_err("must fail");

        assert_eq!(error.status, StatusCode::NOT_FOUND);
        assert!(error.message.contains("ghost"));
    }

    #[tokio::test]
    async fn empty_model_falls_back_to_the_default_model() {
        let state_fixture = fixture();
        let imported = import_model(&state_fixture);
        let engine: Arc<dyn InferenceEngine> = Arc::new(MirrorEngine);
        let resolved = resolve_model(&engine, &state_fixture.catalog, &imported.id, "")
            .await
            .expect("resolve");

        assert_eq!(resolved, "tiny");
    }

    #[tokio::test]
    async fn echo_engine_accepts_any_model_label() {
        let state_fixture = fixture();
        let engine: Arc<dyn InferenceEngine> = Arc::new(crate::inference::EchoEngine);
        let resolved = resolve_model(&engine, &state_fixture.catalog, "", "llama3")
            .await
            .expect("resolve");

        assert_eq!(resolved, "llama3");
    }

    #[test]
    fn tags_response_maps_catalog_entries() {
        let state_fixture = fixture();
        let imported = import_model(&state_fixture);
        let tags = tags_from(state_fixture.catalog.list().expect("list"));

        assert_eq!(tags.models.len(), 1);
        let tag = &tags.models[0];
        assert_eq!(tag.name, "tiny");
        assert_eq!(tag.model, "tiny");
        assert_eq!(tag.digest, imported.id);
        assert_eq!(tag.size, imported.size);
        assert!(tag.modified_at.ends_with('Z'));
    }

    #[test]
    fn show_response_is_minimal_and_catalog_backed() {
        let state_fixture = fixture();
        let imported = import_model(&state_fixture);
        let show = show_from(&imported);

        assert!(show.modelfile.is_empty());
        assert!(show.parameters.is_empty());
        assert!(show.template.is_empty());
        assert_eq!(show.details.format, "wcpu");
        assert_eq!(show.details.family, "weightc");
        assert_eq!(show.details.families, vec!["weightc".to_owned()]);
    }

    #[test]
    fn error_serializes_in_the_ollama_shape() {
        let response = OllamaError::not_found("missing").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = serde_json::to_value(OllamaErrorBody {
            error: "missing".to_owned(),
        })
        .expect("serialize");
        assert_eq!(json, serde_json::json!({"error": "missing"}));
    }

    #[test]
    fn rfc3339_formats_known_timestamps() {
        assert_eq!(rfc3339_from_millis(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339_from_millis(1_700_000_000_000),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(rfc3339_from_millis(951_782_400_000), "2000-02-29T00:00:00Z");
    }
}
