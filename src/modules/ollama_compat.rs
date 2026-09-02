//! Ollama-compatible HTTP API.
//!
//! A thin translation layer over the Phase-1 inference core, mirroring
//! `openai_compat`. Ollama's own API *defaults* to `stream: true`, so
//! `/api/generate` and `/api/chat` stream NDJSON lines (one JSON object per
//! `\n`-terminated line) rather than rejecting it; the `x-hologram-stream`
//! response header discloses whether deltas were real or emulated.

use crate::app::AppState;
use crate::error::LiveError;
use crate::inference::{
    CompletionEvent, CompletionRequest, CompletionStream, CompletionSummary, InferenceEngine,
    StreamKind,
};
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
    /// Omitted unless the engine measured both halves (D3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatResponse {
    pub model: String,
    pub created_at: String,
    pub message: OllamaMessage,
    pub done: bool,
    /// Omitted unless the engine measured both halves (D3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u64>,
}

/// One NDJSON line. `/api/generate` carries `response`; `/api/chat` carries
/// `message`. Exactly one is present per line.
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamLine {
    pub model: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<OllamaMessage>,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u64>,
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
        StreamLine,
        TagsResponse,
        TagModel,
        ShowRequest,
        ShowResponse,
        ModelDetails,
        OllamaErrorBody
    )),
    tags((name = "ollama-compat", description = "Ollama-compatible API"))
)]
struct OllamaApiDoc;

#[utoipa::path(
    post,
    path = "/api/generate",
    request_body = GenerateRequest,
    responses(
        (status = 200, description = "`stream` omitted or `false` returns one JSON `GenerateResponse`. `stream: true` returns `application/x-ndjson` — one `\\n`-terminated `StreamLine` object per line, the last carrying `done: true`. A mid-stream engine failure is reported as one `{\"error\": \"...\"}` NDJSON line, since the status is already 200 by the time streaming starts.", content(
            (GenerateResponse = "application/json"),
            (StreamLine = "application/x-ndjson")
        ), headers(
            ("x-hologram-stream" = String, description = "`native` when the engine produced deltas as it generated them, or `emulated` when the daemon completed the request first and replayed the result as deltas. Present on both streaming and non-streaming responses.")
        )),
        (status = 400, description = "Malformed request or an engine configuration error.", body = OllamaErrorBody),
        (status = 404, body = OllamaErrorBody)
    )
)]
pub async fn generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Response, OllamaError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    dispatch_generate(engine, catalog, &default_model, request).await
}

/// Chooses the streaming or buffered path for `/api/generate`, kept free of
/// `AppState` so unit tests can drive it with a bare engine and catalog.
/// Deleting the `stream == Some(true)` branch here silently reverts the
/// endpoint to buffered-only while every other test stays green, which is
/// why this dispatch is exercised directly rather than only through
/// `generate_core`/`stream_generate`.
async fn dispatch_generate(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: GenerateRequest,
) -> Result<Response, OllamaError> {
    if request.stream == Some(true) {
        return stream_generate(engine, catalog, default_model, request).await;
    }
    // Captured before `generate_core` consumes `engine`: the header must
    // name the engine that served the request regardless of mode, matching
    // the streaming path's `x-hologram-stream` marker (README.md).
    let kind = engine.stream_kind();
    let response = generate_core(engine, catalog, default_model, request).await?;
    let mut response = Json(response).into_response();
    response.headers_mut().insert(
        "x-hologram-stream",
        axum::http::HeaderValue::from_static(kind.header_value()),
    );
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/chat",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "`stream` omitted or `false` returns one JSON `ChatResponse`. `stream: true` returns `application/x-ndjson` — one `\\n`-terminated `StreamLine` object per line, the last carrying `done: true`. A mid-stream engine failure is reported as one `{\"error\": \"...\"}` NDJSON line, since the status is already 200 by the time streaming starts.", content(
            (ChatResponse = "application/json"),
            (StreamLine = "application/x-ndjson")
        ), headers(
            ("x-hologram-stream" = String, description = "`native` when the engine produced deltas as it generated them, or `emulated` when the daemon completed the request first and replayed the result as deltas. Present on both streaming and non-streaming responses.")
        )),
        (status = 400, description = "Empty `messages`, malformed request, or an engine configuration error.", body = OllamaErrorBody),
        (status = 404, body = OllamaErrorBody)
    )
)]
pub async fn chat(
    State(state): State<AppState>,
    Json(request): Json<ChatRequest>,
) -> Result<Response, OllamaError> {
    let engine = state.chat().engine().clone();
    let catalog = state.models().clone();
    let default_model = state.config().inference.default_model.clone();
    dispatch_chat(engine, catalog, &default_model, request).await
}

/// Chooses the streaming or buffered path for `/api/chat`, kept free of
/// `AppState` so unit tests can drive it with a bare engine and catalog.
/// Deleting the `stream == Some(true)` branch here silently reverts the
/// endpoint to buffered-only while every other test stays green, which is
/// why this dispatch is exercised directly rather than only through
/// `chat_core`/`stream_chat`.
async fn dispatch_chat(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: ChatRequest,
) -> Result<Response, OllamaError> {
    if request.stream == Some(true) {
        return stream_chat(engine, catalog, default_model, request).await;
    }
    // Captured before `chat_core` consumes `engine`: the header must name
    // the engine that served the request regardless of mode, matching the
    // streaming path's `x-hologram-stream` marker (README.md).
    let kind = engine.stream_kind();
    let response = chat_core(engine, catalog, default_model, request).await?;
    let mut response = Json(response).into_response();
    response.headers_mut().insert(
        "x-hologram-stream",
        axum::http::HeaderValue::from_static(kind.header_value()),
    );
    Ok(response)
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
    let completion = engine
        .complete(completion_request(request.prompt, request.options))
        .await
        .map_err(OllamaError::from)?;
    Ok(GenerateResponse {
        model,
        created_at: rfc3339_now(),
        response: completion.text,
        done: true,
        prompt_eval_count: completion.usage.map(|usage| usage.prompt_tokens),
        eval_count: completion.usage.map(|usage| usage.completion_tokens),
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
    let completion = engine
        .complete(completion_request(
            render_prompt(&request.messages),
            request.options,
        ))
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
        prompt_eval_count: completion.usage.map(|usage| usage.prompt_tokens),
        eval_count: completion.usage.map(|usage| usage.completion_tokens),
    })
}

/// Builds a `CompletionRequest` from Ollama's `options` object. Shared by the
/// buffered and streaming paths of both `generate` and `chat` so they cannot
/// drift on how `num_predict`/`temperature`/`seed` map onto the engine call.
fn completion_request(prompt: String, options: Option<OllamaOptions>) -> CompletionRequest {
    let options = options.unwrap_or_default();
    CompletionRequest {
        prompt,
        max_tokens: options.num_predict,
        temperature: options.temperature,
        seed: options.seed,
        session_key: None,
    }
}

/// Streaming half of `generate`, kept free of `AppState` so tests can drive
/// it with a bare engine and catalog. Validates the model and starts the
/// engine's stream *before* returning a response: once a body is returned,
/// the 200 is already committed, so an unknown model must fail here rather
/// than mid-stream.
async fn stream_generate(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: GenerateRequest,
) -> Result<Response, OllamaError> {
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let kind = engine.stream_kind();
    let events = engine
        .complete_stream(completion_request(request.prompt, request.options))
        .await
        .map_err(OllamaError::from)?;
    Ok(ndjson_response(kind, model, events, LineShape::Response))
}

/// Streaming half of `chat`, mirroring `stream_generate`.
async fn stream_chat(
    engine: Arc<dyn InferenceEngine>,
    catalog: Arc<ModelCatalog>,
    default_model: &str,
    request: ChatRequest,
) -> Result<Response, OllamaError> {
    if request.messages.is_empty() {
        return Err(OllamaError::bad_request("messages must not be empty"));
    }
    let model = resolve_model(&engine, &catalog, default_model, &request.model).await?;
    let kind = engine.stream_kind();
    let events = engine
        .complete_stream(completion_request(
            render_prompt(&request.messages),
            request.options,
        ))
        .await
        .map_err(OllamaError::from)?;
    Ok(ndjson_response(kind, model, events, LineShape::Message))
}

/// Which field carries the text: `/api/generate` uses `response`,
/// `/api/chat` uses `message`.
#[derive(Debug, Clone, Copy)]
enum LineShape {
    Response,
    Message,
}

/// Drives `events` to completion on a background task, emitting one
/// `\n`-terminated NDJSON line per item on `sender`, and wraps the receiving
/// half as the streamed response body.
fn ndjson_response(
    kind: StreamKind,
    model: String,
    events: CompletionStream,
    shape: LineShape,
) -> Response {
    let (sender, receiver) =
        tokio::sync::mpsc::channel::<Result<String, std::convert::Infallible>>(16);

    tokio::spawn(async move {
        use tokio_stream::StreamExt;

        let line = |text: Option<String>, done: bool, summary: Option<CompletionSummary>| {
            let (response, message) = match (shape, text) {
                (_, None) => (None, None),
                (LineShape::Response, Some(text)) => (Some(text), None),
                (LineShape::Message, Some(text)) => (
                    None,
                    Some(OllamaMessage {
                        role: "assistant".to_owned(),
                        content: text,
                    }),
                ),
            };
            let usage = summary.as_ref().and_then(|summary| summary.usage);
            let value = StreamLine {
                model: model.clone(),
                created_at: rfc3339_now(),
                response,
                message,
                done,
                done_reason: done.then(|| "stop".to_owned()),
                prompt_eval_count: usage.map(|usage| usage.prompt_tokens),
                eval_count: usage.map(|usage| usage.completion_tokens),
            };
            format!("{}\n", serde_json::to_string(&value).unwrap_or_default())
        };

        // `.send().await` applies backpressure; `try_send` would error (and
        // read as a disconnect) the moment a slow client let the 16-slot
        // channel fill, silently truncating the stream.
        let mut events = events;
        let mut summary = None;
        while let Some(event) = events.next().await {
            match event {
                Ok(CompletionEvent::Delta(text)) => {
                    if sender.send(Ok(line(Some(text), false, None))).await.is_err() {
                        return;
                    }
                }
                Ok(CompletionEvent::Done(done)) => {
                    // The engine contract promises `Done` is terminal, but an
                    // engine that kept yielding afterward would otherwise
                    // hang the client with no second terminal line. Breaking
                    // here enforces "exactly one `Done`, then the stream
                    // ends" rather than trusting it.
                    summary = Some(done);
                    break;
                }
                Err(error) => {
                    // Status is already 200, so in-band is the only honest
                    // way to report this (§4). Return rather than fall
                    // through to the terminal `done: true` line below: doing
                    // so would claim a clean completion that did not happen.
                    let envelope = OllamaErrorBody {
                        error: error.to_string(),
                    };
                    let encoded = serde_json::to_string(&envelope).unwrap_or_default();
                    let _ = sender.send(Ok(format!("{encoded}\n"))).await;
                    return;
                }
            }
        }
        let _ = sender.send(Ok(line(None, true, summary))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(receiver);
    let mut response = axum::body::Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/x-ndjson"),
    );
    response.headers_mut().insert(
        "x-hologram-stream",
        axum::http::HeaderValue::from_static(kind.header_value()),
    );
    response
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
                usage: None,
            })
        }

        async fn list_models(&self) -> crate::error::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    /// Streams one delta, then fails. Only a native engine can fail this way;
    /// buffered engines resolve `complete()` before the stream exists.
    struct HalfwayFailingEngine;

    #[tonic::async_trait]
    impl InferenceEngine for HalfwayFailingEngine {
        fn name(&self) -> &'static str {
            "halfway-failing"
        }

        fn stream_kind(&self) -> StreamKind {
            StreamKind::Native
        }

        async fn complete(&self, _request: CompletionRequest) -> crate::error::Result<Completion> {
            Err(LiveError::Transport("this fixture only streams".to_owned()))
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> crate::error::Result<CompletionStream> {
            Ok(Box::pin(tokio_stream::iter(vec![
                Ok(CompletionEvent::Delta("Hel".to_owned())),
                Err(LiveError::Transport("the engine vanished".to_owned())),
            ])))
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

    /// Reads a streamed NDJSON response body and parses each
    /// `\n`-terminated line as JSON, in arrival order. Parsed JSON (rather
    /// than substring search) is what lets a test prove a field's *absence*
    /// and inspect a line's position within the stream.
    async fn ndjson_lines(response: Response) -> Vec<serde_json::Value> {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read the streamed body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf-8");
        body.lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("line is valid JSON ({error}): {line}"))
            })
            .collect()
    }

    /// Was `stream_true_is_rejected`. Ollama's own API defaults to
    /// stream: true, so rejecting it broke that ecosystem's default path.
    #[tokio::test]
    async fn generate_streams_ndjson_lines_and_marks_emulation() {
        let fixture = fixture();
        let response = stream_generate(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "Hello".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the echo engine streams by emulation");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "emulated"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let lines = ndjson_lines(response).await;
        assert_eq!(
            lines.len(),
            2,
            "one delta line, then one terminal line: {lines:?}"
        );

        // Positional, not "somewhere in the body": index 0 must be the
        // delta, and the last index must be the terminal line.
        assert_eq!(lines[0]["response"], serde_json::json!("Hello"));
        assert_eq!(lines[0]["done"], serde_json::json!(false));
        assert!(
            lines[0].get("message").is_none(),
            "generate lines never carry a chat message: {:?}",
            lines[0]
        );

        let terminal = lines.last().expect("a terminal line");
        assert_eq!(terminal["done"], serde_json::json!(true));
        assert_eq!(terminal["done_reason"], serde_json::json!("stop"));
        assert!(
            terminal.get("response").is_none(),
            "the terminal line carries no further text: {terminal:?}"
        );
        assert!(
            terminal.get("prompt_eval_count").is_none(),
            "echo measures nothing, so counts are absent rather than zero: {terminal:?}"
        );
        assert!(terminal.get("eval_count").is_none());
    }

    /// Mirrors the generate test above, but for `/api/chat`: the text lands
    /// in `message: {role, content}` rather than `response`.
    #[tokio::test]
    async fn chat_streams_ndjson_lines_and_marks_emulation() {
        let fixture = fixture();
        let response = stream_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            ChatRequest {
                model: String::new(),
                messages: vec![OllamaMessage {
                    role: "user".to_owned(),
                    content: "Hello".to_owned(),
                }],
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the echo engine streams by emulation");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "emulated"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let lines = ndjson_lines(response).await;
        assert_eq!(
            lines.len(),
            2,
            "one delta line, then one terminal line: {lines:?}"
        );

        assert_eq!(lines[0]["message"]["role"], serde_json::json!("assistant"));
        assert_eq!(lines[0]["message"]["content"], serde_json::json!("Hello"));
        assert_eq!(lines[0]["done"], serde_json::json!(false));
        assert!(
            lines[0].get("response").is_none(),
            "chat lines never carry the generate-shaped response field: {:?}",
            lines[0]
        );

        let terminal = lines.last().expect("a terminal line");
        assert_eq!(terminal["done"], serde_json::json!(true));
        assert_eq!(terminal["done_reason"], serde_json::json!("stop"));
        assert!(terminal.get("message").is_none());
    }

    /// Proves both that a native engine's mid-stream failure is reported
    /// in-band on `/api/generate` (§4: the status is already 200, so this is
    /// the only honest way to report it) and that `x-hologram-stream` is
    /// genuinely derived from `stream_kind()` rather than hardcoded — every
    /// other fixture in this module is `Buffered`, so a hardcoded "emulated"
    /// would pass every other assertion in this file.
    #[tokio::test]
    async fn generate_mid_stream_failure_is_reported_in_band_and_marked_native() {
        let fixture = fixture();
        let response = stream_generate(
            Arc::new(HalfwayFailingEngine),
            fixture.catalog.clone(),
            "",
            GenerateRequest {
                model: String::new(),
                prompt: "Hello".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the stream opens before the failure occurs");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "native"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let lines = ndjson_lines(response).await;
        assert_eq!(
            lines.len(),
            2,
            "one delta line, then one error line: {lines:?}"
        );

        assert_eq!(lines[0]["response"], serde_json::json!("Hel"));
        assert_eq!(lines[0]["done"], serde_json::json!(false));

        let last = lines.last().expect("a final line");
        assert!(
            last.get("error").is_some(),
            "the failure is reported in-band as an error line: {last:?}"
        );

        // Proves absence, not just that some other line is present: a
        // failed stream must never claim `done: true` anywhere in the body
        // — that would let a swallowed error masquerade as a clean finish.
        assert!(
            lines.iter().all(|line| line["done"] != serde_json::json!(true)),
            "no line may claim done: true after a mid-stream failure: {lines:?}"
        );
    }

    /// Mirrors the generate test above, but for `/api/chat` — the NDJSON
    /// streaming path distinct from the SSE `stream_chat` in
    /// `openai_compat.rs`.
    #[tokio::test]
    async fn chat_mid_stream_failure_is_reported_in_band_and_marked_native() {
        let fixture = fixture();
        let response = stream_chat(
            Arc::new(HalfwayFailingEngine),
            fixture.catalog.clone(),
            "",
            ChatRequest {
                model: String::new(),
                messages: vec![OllamaMessage {
                    role: "user".to_owned(),
                    content: "Hello".to_owned(),
                }],
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the stream opens before the failure occurs");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is always present")
                .to_str()
                .expect("ascii"),
            "native"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let lines = ndjson_lines(response).await;
        assert_eq!(
            lines.len(),
            2,
            "one delta line, then one error line: {lines:?}"
        );

        assert_eq!(lines[0]["message"]["content"], serde_json::json!("Hel"));
        assert_eq!(lines[0]["done"], serde_json::json!(false));

        let last = lines.last().expect("a final line");
        assert!(
            last.get("error").is_some(),
            "the failure is reported in-band as an error line: {last:?}"
        );

        assert!(
            lines.iter().all(|line| line["done"] != serde_json::json!(true)),
            "no line may claim done: true after a mid-stream failure: {lines:?}"
        );
    }

    /// `stream_chat` rejects empty messages before touching the engine at
    /// all, same as the non-streaming path.
    #[tokio::test]
    async fn stream_chat_rejects_empty_messages() {
        let fixture = fixture();
        let error = stream_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            ChatRequest {
                model: String::new(),
                messages: Vec::new(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect_err("must fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    /// An unknown model must fail *before* a response is returned: a typed
    /// 404, not a stream that opens and then fails in-band.
    #[tokio::test]
    async fn stream_generate_rejects_an_unknown_model_before_returning_a_body() {
        let fixture = fixture();
        let error = stream_generate(
            Arc::new(MirrorEngine),
            fixture.catalog.clone(),
            "",
            GenerateRequest {
                model: "ghost".to_owned(),
                prompt: "hi".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect_err("must fail");

        assert_eq!(error.status, StatusCode::NOT_FOUND);
    }

    /// Reports fixed, distinct prompt/completion counts so a transposition
    /// bug would fail this test even though both fields would still
    /// serialize, and checks that they land specifically on the terminal
    /// line rather than merely somewhere in the body.
    #[tokio::test]
    async fn streaming_reports_measured_counts_on_the_terminal_line() {
        let fixture = fixture();
        let response = stream_generate(
            Arc::new(MeteredEngine),
            fixture.catalog.clone(),
            "",
            GenerateRequest {
                model: String::new(),
                prompt: "hello".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the metered engine streams by emulation");

        let lines = ndjson_lines(response).await;
        let terminal = lines.last().expect("a terminal line");
        assert_eq!(terminal["done"], serde_json::json!(true));
        assert_eq!(terminal["prompt_eval_count"], serde_json::json!(11));
        assert_eq!(terminal["eval_count"], serde_json::json!(22));

        // Confirms the counts are absent from every non-terminal line.
        for (index, line) in lines[..lines.len() - 1].iter().enumerate() {
            assert!(
                line.get("prompt_eval_count").is_none() && line.get("eval_count").is_none(),
                "line {index} is not terminal and must carry no counts: {line:?}"
            );
        }
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

    #[tokio::test]
    async fn generate_omits_counts_when_the_engine_measures_none() {
        let fixture = fixture();
        let response = generate_core(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "Hello".to_owned(),
                stream: Some(false),
                options: None,
            },
        )
        .await
        .expect("the echo engine always completes");

        let encoded = serde_json::to_value(&response).expect("serialize");
        assert!(encoded.get("eval_count").is_none());
        assert!(encoded.get("prompt_eval_count").is_none());
    }

    /// Reports fixed, distinct usage so a prompt/completion transposition
    /// bug (D3) would fail this test even though both fields would still
    /// serialize.
    struct MeteredEngine;

    #[tonic::async_trait]
    impl InferenceEngine for MeteredEngine {
        fn name(&self) -> &'static str {
            "metered"
        }

        async fn complete(&self, request: CompletionRequest) -> crate::error::Result<Completion> {
            Ok(Completion {
                text: request.prompt,
                model: "metered".to_owned(),
                tokens_per_second: None,
                elapsed_millis: 0,
                usage: Some(crate::inference::TokenUsage {
                    prompt_tokens: 11,
                    completion_tokens: 22,
                }),
            })
        }

        async fn list_models(&self) -> crate::error::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn generate_reports_measured_counts_with_the_ollama_field_names() {
        let fixture = fixture();
        let response = generate_core(
            Arc::new(MeteredEngine),
            fixture.catalog.clone(),
            "",
            generate_request("", "hello"),
        )
        .await
        .expect("generate");

        assert_eq!(response.prompt_eval_count, Some(11));
        assert_eq!(response.eval_count, Some(22));
    }

    #[tokio::test]
    async fn chat_reports_measured_counts_with_the_ollama_field_names() {
        let fixture = fixture();
        let request = ChatRequest {
            model: String::new(),
            messages: vec![OllamaMessage {
                role: "user".to_owned(),
                content: "ping".to_owned(),
            }],
            stream: Some(false),
            options: None,
        };
        let response = chat_core(Arc::new(MeteredEngine), fixture.catalog.clone(), "", request)
            .await
            .expect("chat");

        assert_eq!(response.prompt_eval_count, Some(11));
        assert_eq!(response.eval_count, Some(22));

        let encoded = serde_json::to_value(&response).expect("serialize");
        assert_eq!(encoded["prompt_eval_count"], serde_json::json!(11));
        assert_eq!(encoded["eval_count"], serde_json::json!(22));
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

    /// README.md and the `utoipa` annotations both promise `x-hologram-stream`
    /// "on both streaming and non-streaming responses"; before this test (and
    /// the corresponding fix) the header was only ever set on the NDJSON
    /// streaming path.
    #[tokio::test]
    async fn non_streaming_generate_carries_the_stream_header() {
        let fixture = fixture();
        let response = dispatch_generate(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            generate_request("", "hello"),
        )
        .await
        .expect("the echo engine always completes");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is present on non-streaming responses too")
                .to_str()
                .expect("ascii"),
            "emulated"
        );
    }

    /// Mirrors `non_streaming_generate_carries_the_stream_header` for
    /// `/api/chat`.
    #[tokio::test]
    async fn non_streaming_chat_carries_the_stream_header() {
        let fixture = fixture();
        let request = ChatRequest {
            model: String::new(),
            messages: vec![OllamaMessage {
                role: "user".to_owned(),
                content: "ping".to_owned(),
            }],
            stream: Some(false),
            options: None,
        };
        let response = dispatch_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            request,
        )
        .await
        .expect("the echo engine always completes");

        assert_eq!(
            response
                .headers()
                .get("x-hologram-stream")
                .expect("the marker is present on non-streaming responses too")
                .to_str()
                .expect("ascii"),
            "emulated"
        );
    }

    /// Exercises the `stream == Some(true)` branch in `dispatch_generate`
    /// directly (the code the public `generate` handler reduces to after
    /// `AppState` extraction), since no test otherwise touches the handler
    /// itself: deleting the branch would silently revert the endpoint to
    /// buffered-only while every other test stayed green.
    #[tokio::test]
    async fn dispatch_generate_streams_or_not_based_on_the_request() {
        let fixture = fixture();

        let streaming_response = dispatch_generate(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "hello".to_owned(),
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the echo engine streams by emulation");
        assert_eq!(
            streaming_response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let buffered_response = dispatch_generate(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            GenerateRequest {
                model: String::new(),
                prompt: "hello".to_owned(),
                stream: None,
                options: None,
            },
        )
        .await
        .expect("the echo engine always completes");
        assert_eq!(
            buffered_response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/json"
        );
    }

    /// Mirrors `dispatch_generate_streams_or_not_based_on_the_request` for
    /// `/api/chat`.
    #[tokio::test]
    async fn dispatch_chat_streams_or_not_based_on_the_request() {
        let fixture = fixture();
        let message = OllamaMessage {
            role: "user".to_owned(),
            content: "hello".to_owned(),
        };

        let streaming_response = dispatch_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            ChatRequest {
                model: String::new(),
                messages: vec![message.clone()],
                stream: Some(true),
                options: None,
            },
        )
        .await
        .expect("the echo engine streams by emulation");
        assert_eq!(
            streaming_response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/x-ndjson"
        );

        let buffered_response = dispatch_chat(
            Arc::new(crate::inference::EchoEngine),
            fixture.catalog.clone(),
            "echo",
            ChatRequest {
                model: String::new(),
                messages: vec![message],
                stream: None,
                options: None,
            },
        )
        .await
        .expect("the echo engine always completes");
        assert_eq!(
            buffered_response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content type is set")
                .to_str()
                .expect("ascii"),
            "application/json"
        );
    }
}
