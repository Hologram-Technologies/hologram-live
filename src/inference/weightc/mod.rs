//! One-shot CLI engine: `weightc ask <artifact-dir> <prompt> --json`.
//!
//! With `inference.resident_sessions = true`, requests carrying a
//! `session_key` are served by resident `weightc enter --jsonl` children, one
//! per key, bounded by `inference.max_resident_sessions` with LRU eviction.
//! The resident child holds the KV context, so only the new turn is sent.
//! When a child dies or breaks the protocol, the request that discovers it
//! fails with a typed error naming the lost session; the next request on that
//! key lazily spawns a fresh session whose context starts over.

mod session;

use self::session::{
    SessionTable, SessionTurn, StopSession, WeightcSessionActor, SHUTDOWN_TIMEOUT,
};
use super::{Completion, CompletionRequest, InferenceEngine, TokenUsage};
use crate::actor::ActorSystem;
use crate::config::InferenceConfig;
use crate::error::{LiveError, Result};
use crate::models::{ModelCatalog, ModelInfo};
use kameo::actor::ActorRef;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// One-shot CLI engine over `weightc`. See module docs for the resident
/// session behavior.
pub struct WeightcEngine {
    binary: String,
    timeout: Duration,
    catalog: Arc<ModelCatalog>,
    default_model: String,
    resident_sessions: bool,
    max_sessions: usize,
    mailbox_capacity: usize,
    sessions: Mutex<SessionTable>,
    actors: OnceLock<ActorSystem>,
}

impl WeightcEngine {
    pub fn new(
        config: &InferenceConfig,
        catalog: Arc<ModelCatalog>,
        mailbox_capacity: usize,
    ) -> Self {
        Self {
            binary: config.weightc_path.clone(),
            timeout: Duration::from_secs(config.request_timeout_secs),
            catalog,
            default_model: config.default_model.clone(),
            resident_sessions: config.resident_sessions,
            max_sessions: config.max_resident_sessions.max(1),
            mailbox_capacity: mailbox_capacity.max(1),
            sessions: Mutex::new(SessionTable::default()),
            actors: OnceLock::new(),
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

    async fn complete_resident(
        &self,
        key: &str,
        request: &CompletionRequest,
    ) -> Result<Completion> {
        let actor = self.session_actor(key, request).await?;
        let turn = SessionTurn {
            prompt: request.prompt.clone(),
            system: None,
        };
        match actor.ask(turn).await {
            Ok(outcome) => {
                if !outcome.alive {
                    // The session broke; drop the actor so the next request
                    // on this key starts a fresh session.
                    self.sessions.lock().await.remove(key);
                }
                outcome.result
            }
            Err(error) => {
                self.sessions.lock().await.remove(key);
                Err(LiveError::Transport(format!(
                    "weightc session {key} actor is unavailable: {error}"
                )))
            }
        }
    }

    async fn session_actor(
        &self,
        key: &str,
        request: &CompletionRequest,
    ) -> Result<ActorRef<WeightcSessionActor>> {
        let mut table = self.sessions.lock().await;
        if let Some(actor) = table.get(key) {
            return Ok(actor);
        }
        let spec = SessionSpec {
            binary: self.binary.clone(),
            artifact: self.artifact_dir()?,
            sampling: sampling_args(request),
            timeout: self.timeout,
            model: self.default_model.clone(),
        };
        let supervisor = self.actors.get_or_init(ActorSystem::start).root().clone();
        Ok(table
            .spawn(
                key,
                spec,
                self.max_sessions,
                &supervisor,
                self.mailbox_capacity,
            )
            .await)
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
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[tonic::async_trait]
impl InferenceEngine for WeightcEngine {
    fn name(&self) -> &'static str {
        "weightc"
    }

    fn supports_sessions(&self) -> bool {
        self.resident_sessions
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        if self.resident_sessions {
            if let Some(key) = request.session_key.clone() {
                return self.complete_resident(&key, &request).await;
            }
        }
        self.complete_one_shot(&request).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.catalog.list()
    }

    /// Stop every resident session actor, which kills its child process. The
    /// `kill_on_drop` child handle is the backstop for abrupt daemon exits.
    async fn shutdown(&self) {
        let actors = self.sessions.lock().await.drain();
        for actor in &actors {
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, actor.ask(StopSession)).await;
            let _ = actor.stop_gracefully().await;
        }
        for actor in &actors {
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, actor.wait_for_shutdown()).await;
        }
    }
}

impl WeightcEngine {
    async fn complete_one_shot(&self, request: &CompletionRequest) -> Result<Completion> {
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
                super::stderr_tail(stderr.trim())
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
            elapsed_millis: super::elapsed_millis(started),
            usage: TokenUsage::from_counts(parsed.prompt_tokens, parsed.completion_tokens),
        })
    }
}

/// Everything a session actor needs to (re)spawn its `weightc enter` child.
struct SessionSpec {
    binary: String,
    artifact: PathBuf,
    /// Sampling flags from the first turn's request; fixed for the life of
    /// the session.
    sampling: Vec<String>,
    timeout: Duration,
    model: String,
}

/// Sampling flags fixed at session spawn: only what the first turn's
/// request carries (`--max-tokens`, `--temperature`, `--seed`).
fn sampling_args(request: &CompletionRequest) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(max_tokens) = request.max_tokens {
        args.push("--max-tokens".to_owned());
        args.push(max_tokens.to_string());
    }
    if let Some(temperature) = request.temperature {
        args.push("--temperature".to_owned());
        args.push(temperature.to_string());
    }
    if let Some(seed) = request.seed {
        args.push("--seed".to_owned());
        args.push(seed.to_string());
    }
    args
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::models::ModelCatalog;
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
        let store = Arc::new(crate::store::ObjectStore::open(root.join("store")).expect("store"));
        let catalog = Arc::new(ModelCatalog::open(store, root.join("models")).expect("catalog"));
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

    fn prompt(text: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: text.to_owned(),
            ..CompletionRequest::default()
        }
    }

    #[tokio::test]
    async fn weightc_parses_the_json_response() {
        let fixture = weightc_fixture(
            "#!/bin/sh\nprintf '{\"response\": \"weightc answer\", \"tokens_per_second\": 12.5}\\n'\n",
        );
        let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone(), 8);
        let completion = engine.complete(prompt("user: hi")).await.expect("complete");
        assert_eq!(completion.text, "weightc answer");
        assert_eq!(completion.tokens_per_second, Some(12.5));
        assert_eq!(engine.name(), "weightc");
        assert_eq!(engine.list_models().await.expect("models").len(), 1);
    }

    #[tokio::test]
    async fn weightc_accepts_alternate_text_fields() {
        let fixture = weightc_fixture("#!/bin/sh\nprintf '{\"text\": \"from text field\"}\\n'\n");
        let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone(), 8);
        let completion = engine.complete(prompt("user: hi")).await.expect("complete");
        assert_eq!(completion.text, "from text field");
    }

    #[tokio::test]
    async fn weightc_nonzero_exit_reports_the_stderr_tail() {
        let fixture = weightc_fixture("#!/bin/sh\necho boom >&2\nexit 3\n");
        let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone(), 8);
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
        let engine = WeightcEngine::new(&config, fixture.catalog.clone(), 8);
        let error = engine
            .complete(prompt("user: hi"))
            .await
            .expect_err("must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
    }
}
