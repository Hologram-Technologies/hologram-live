//! Inference engine boundary.
//!
//! The daemon never executes model weights in-process. Chat (and later the
//! OpenAI/Ollama-compatible modules) call an [`InferenceEngine`]; engines
//! either echo locally or delegate to an external engine — the `weightc`
//! one-shot CLI over `.wcpu` artifact directories, or an Ollama-compatible
//! HTTP endpoint.

use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::{ModelCatalog, ModelInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub tokens_per_second: Option<f64>,
    pub elapsed_millis: u64,
}

#[tonic::async_trait]
pub trait InferenceEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn complete(&self, request: CompletionRequest) -> Result<Completion>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

pub fn engine_from_config(
    config: &InferenceConfig,
    catalog: Arc<ModelCatalog>,
) -> Result<Arc<dyn InferenceEngine>> {
    match config.engine.as_str() {
        "echo" => Ok(Arc::new(EchoEngine)),
        "weightc" => Ok(Arc::new(WeightcEngine::new(config, catalog))),
        "ollama" => Ok(Arc::new(OllamaEngine::new(config)?)),
        other => Err(LiveError::Config(format!(
            "unsupported inference.engine {other:?}; expected echo, weightc, or ollama"
        ))),
    }
}

/// Local fallback engine: the assistant response repeats the user's message.
pub struct EchoEngine;

#[tonic::async_trait]
impl InferenceEngine for EchoEngine {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let started = Instant::now();
        Ok(Completion {
            text: last_user_content(&request.prompt).to_owned(),
            model: "echo".to_owned(),
            tokens_per_second: None,
            elapsed_millis: elapsed_millis(started),
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }
}

/// Newest user turn of a rendered `role: content` transcript. Prompts without
/// transcript framing are returned unchanged, preserving the original
/// echo-demo behavior for a bare single-turn prompt.
fn last_user_content(prompt: &str) -> &str {
    for line in prompt.lines().rev() {
        if let Some(content) = line.strip_prefix("user: ") {
            return content;
        }
    }
    prompt.trim_end()
}

/// One-shot CLI engine: `weightc ask <artifact-dir> <prompt> --json`.
pub struct WeightcEngine {
    binary: String,
    timeout: Duration,
    catalog: Arc<ModelCatalog>,
    default_model: String,
}

impl WeightcEngine {
    pub fn new(config: &InferenceConfig, catalog: Arc<ModelCatalog>) -> Self {
        Self {
            binary: config.weightc_path.clone(),
            timeout: Duration::from_secs(config.request_timeout_secs),
            catalog,
            default_model: config.default_model.clone(),
        }
    }

    fn artifact_dir(&self) -> Result<PathBuf> {
        if self.default_model.trim().is_empty() {
            return Err(LiveError::Capability(
                "the weightc engine requires inference.default_model to name an imported model"
                    .to_owned(),
            ));
        }
        self.catalog.artifact_dir(&self.default_model)
    }
}

/// `weightc ask --json` emits one JSON object on stdout. The response text
/// field name is accepted defensively because the CLI contract is young.
#[derive(Debug, Deserialize)]
struct WeightcAskOutput {
    response: Option<String>,
    text: Option<String>,
    output: Option<String>,
    tokens_per_second: Option<f64>,
}

#[tonic::async_trait]
impl InferenceEngine for WeightcEngine {
    fn name(&self) -> &'static str {
        "weightc"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let artifact = self.artifact_dir()?;
        let started = Instant::now();
        let output = tokio::time::timeout(
            self.timeout,
            tokio::process::Command::new(&self.binary)
                .arg("ask")
                .arg(&artifact)
                .arg(&request.prompt)
                .arg("--json")
                .output(),
        )
        .await
        .map_err(|_| {
            LiveError::Transport(format!(
                "weightc ask timed out after {}s",
                self.timeout.as_secs()
            ))
        })?
        .map_err(|error| {
            LiveError::Transport(format!("failed to start {}: {error}", self.binary))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LiveError::Io(format!(
                "weightc ask failed ({}): {}",
                output.status,
                stderr_tail(stderr.trim())
            )));
        }
        let parsed: WeightcAskOutput = serde_json::from_slice(&output.stdout)?;
        let text = parsed
            .response
            .or(parsed.text)
            .or(parsed.output)
            .ok_or_else(|| {
                LiveError::Protocol(
                    "weightc --json output has no response, text, or output field".to_owned(),
                )
            })?;
        Ok(Completion {
            text,
            model: self.default_model.clone(),
            tokens_per_second: parsed.tokens_per_second,
            elapsed_millis: elapsed_millis(started),
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.catalog.list()
    }
}

/// Ollama-compatible HTTP engine (`POST {endpoint}/api/generate`).
pub struct OllamaEngine {
    endpoint: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaEngine {
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| LiveError::Transport(format!("build ollama client: {error}")))?;
        Ok(Self {
            endpoint: config.ollama_endpoint.trim_end_matches('/').to_owned(),
            model: config.default_model.clone(),
            client,
        })
    }
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    eval_count: Option<u64>,
    eval_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTag>,
}

#[derive(Debug, Deserialize)]
struct OllamaTag {
    name: String,
    #[serde(default)]
    size: u64,
}

#[tonic::async_trait]
impl InferenceEngine for OllamaEngine {
    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        if self.model.trim().is_empty() {
            return Err(LiveError::Capability(
                "the ollama engine requires inference.default_model to name a model tag".to_owned(),
            ));
        }
        let started = Instant::now();
        let options = if request.max_tokens.is_some()
            || request.temperature.is_some()
            || request.seed.is_some()
        {
            Some(OllamaOptions {
                num_predict: request.max_tokens,
                temperature: request.temperature,
                seed: request.seed,
            })
        } else {
            None
        };
        let body = OllamaGenerateRequest {
            model: &self.model,
            prompt: &request.prompt,
            stream: false,
            options,
        };
        let response = self
            .client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("ollama generate: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LiveError::Transport(format!(
                "ollama generate failed ({status}): {}",
                stderr_tail(body.trim())
            )));
        }
        let parsed: OllamaGenerateResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse ollama response: {error}")))?;
        // Rates are informational; sub-token precision is not meaningful here.
        #[allow(clippy::cast_precision_loss)]
        let tokens_per_second = match (parsed.eval_count, parsed.eval_duration) {
            (Some(count), Some(duration)) if duration > 0 => {
                Some(count as f64 / (duration as f64 / 1_000_000_000.0))
            }
            _ => None,
        };
        Ok(Completion {
            text: parsed.response,
            model: self.model.clone(),
            tokens_per_second,
            elapsed_millis: elapsed_millis(started),
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.endpoint))
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("ollama tags: {error}")))?;
        if !response.status().is_success() {
            return Err(LiveError::Transport(format!(
                "ollama tags failed ({})",
                response.status()
            )));
        }
        let parsed: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|error| LiveError::Protocol(format!("parse ollama tags: {error}")))?;
        Ok(parsed
            .models
            .into_iter()
            .map(|tag| ModelInfo {
                id: tag.name.clone(),
                name: tag.name,
                engine: "ollama".to_owned(),
                source: self.endpoint.clone(),
                size: tag.size,
                created_at_millis: 0,
            })
            .collect())
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn stderr_tail(text: &str) -> &str {
    const LIMIT: usize = 512;
    if text.len() <= LIMIT {
        return text;
    }
    let mut start = text.len() - LIMIT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: text.to_owned(),
            ..CompletionRequest::default()
        }
    }

    #[tokio::test]
    async fn echo_returns_the_newest_user_turn() {
        let engine = EchoEngine;
        let completion = engine
            .complete(prompt("user: first\nassistant: first\nuser: second"))
            .await
            .expect("echo");
        assert_eq!(completion.text, "second");
        assert_eq!(engine.name(), "echo");
        assert!(engine.list_models().await.expect("models").is_empty());
    }

    #[tokio::test]
    async fn echo_returns_a_bare_prompt_unchanged() {
        let engine = EchoEngine;
        let completion = engine.complete(prompt("hello")).await.expect("echo");
        assert_eq!(completion.text, "hello");
    }

    #[test]
    fn unknown_engine_is_a_config_error() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let store = Arc::new(
            crate::store::ObjectStore::open(temporary.path().join("store")).expect("store"),
        );
        let catalog =
            Arc::new(ModelCatalog::open(store, temporary.path().join("models")).expect("catalog"));
        let config = InferenceConfig {
            engine: "surprise".to_owned(),
            ..InferenceConfig::default()
        };
        let error = engine_from_config(&config, catalog)
            .err()
            .expect("must fail");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
    }

    #[cfg(unix)]
    mod weightc {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        struct WeightcFixture {
            _temporary: tempfile::TempDir,
            catalog: Arc<ModelCatalog>,
            binary: PathBuf,
            model_id: String,
        }

        /// Installs a fake `weightc` executable (a shell script emitting the
        /// documented `--json` output shape) plus one imported artifact.
        fn weightc_fixture(script: &str) -> WeightcFixture {
            let temporary = tempfile::tempdir().expect("temporary directory");
            let root = temporary.path();
            let binary = root.join("weightc");
            std::fs::write(&binary, script).expect("write fake weightc");
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake weightc");
            let artifact = root.join("tiny.wcpu");
            std::fs::create_dir_all(&artifact).expect("artifact dir");
            std::fs::write(artifact.join("manifest.json"), b"{}").expect("manifest");
            let store =
                Arc::new(crate::store::ObjectStore::open(root.join("store")).expect("store"));
            let catalog =
                Arc::new(ModelCatalog::open(store, root.join("models")).expect("catalog"));
            let model_id = catalog.import(&artifact).expect("import").id;
            WeightcFixture {
                catalog,
                binary,
                model_id,
                _temporary: temporary,
            }
        }

        fn config_for(fixture: &WeightcFixture) -> InferenceConfig {
            InferenceConfig {
                engine: "weightc".to_owned(),
                default_model: fixture.model_id.clone(),
                weightc_path: fixture.binary.to_string_lossy().into_owned(),
                ..InferenceConfig::default()
            }
        }

        #[tokio::test]
        async fn weightc_parses_the_json_response() {
            let fixture = weightc_fixture(
                "#!/bin/sh\nprintf '{\"response\": \"weightc answer\", \"tokens_per_second\": 12.5}\\n'\n",
            );
            let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone());
            let completion = engine.complete(prompt("user: hi")).await.expect("complete");
            assert_eq!(completion.text, "weightc answer");
            assert_eq!(completion.tokens_per_second, Some(12.5));
            assert_eq!(engine.name(), "weightc");
            assert_eq!(engine.list_models().await.expect("models").len(), 1);
        }

        #[tokio::test]
        async fn weightc_accepts_alternate_text_fields() {
            let fixture =
                weightc_fixture("#!/bin/sh\nprintf '{\"text\": \"from text field\"}\\n'\n");
            let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone());
            let completion = engine.complete(prompt("user: hi")).await.expect("complete");
            assert_eq!(completion.text, "from text field");
        }

        #[tokio::test]
        async fn weightc_nonzero_exit_reports_the_stderr_tail() {
            let fixture = weightc_fixture("#!/bin/sh\necho boom >&2\nexit 3\n");
            let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone());
            let error = engine
                .complete(prompt("user: hi"))
                .await
                .expect_err("must fail");
            assert!(error.to_string().contains("boom"), "error: {error}");
        }

        #[tokio::test]
        async fn weightc_without_a_default_model_is_a_capability_error() {
            let fixture = weightc_fixture("#!/bin/sh\nexit 0\n");
            let config = InferenceConfig {
                default_model: String::new(),
                ..config_for(&fixture)
            };
            let engine = WeightcEngine::new(&config, fixture.catalog.clone());
            let error = engine
                .complete(prompt("user: hi"))
                .await
                .expect_err("must fail");
            assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        }
    }
}
