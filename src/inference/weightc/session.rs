//! Resident `weightc enter --jsonl` session actor and session table.
//!
//! One resident `weightc enter --jsonl` child holds a conversation's KV
//! context. The child spawns lazily on the first turn. Each turn writes one
//! JSON request line and reads one receipt or error line back within
//! `inference.request_timeout_secs`; the optional `system` field is written
//! with the first successful turn only.
//!
//! Receipt error lines keep the session alive — the weightc loop continues
//! after them. Child death, a read timeout, or an unparsable line breaks
//! the session: the turn fails with a typed error naming the session key
//! and the actor tears the child down, so the next turn on the key starts a
//! fresh session (context is lost).

use super::SessionSpec;
use crate::actor::RootSupervisor;
use crate::error::{LiveError, Result};
use crate::inference::{elapsed_millis, Completion};
use kameo::actor::{ActorRef, Spawn};
use kameo::mailbox;
use kameo::message::{Context, Message};
use kameo::Actor;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// Grace period for resident session actors to stop during eviction or
/// daemon shutdown.
pub(super) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Resident session actors keyed by session id, with LRU recency tracking.
#[derive(Default)]
pub(super) struct SessionTable {
    actors: HashMap<String, ActorRef<WeightcSessionActor>>,
    recency: VecDeque<String>,
}

impl SessionTable {
    /// The live actor for `key`, marking it most recently used. Dead entries
    /// are dropped so the caller respawns a fresh session.
    pub(super) fn get(&mut self, key: &str) -> Option<ActorRef<WeightcSessionActor>> {
        let actor = self.actors.get(key)?.clone();
        if actor.is_alive() {
            self.touch(key);
            return Some(actor);
        }
        self.remove(key);
        None
    }

    /// Spawn a session actor for `key`, evicting the least recently used
    /// session first when the table is full.
    pub(super) async fn spawn(
        &mut self,
        key: &str,
        spec: SessionSpec,
        max: usize,
        supervisor: &ActorRef<RootSupervisor>,
        mailbox_capacity: usize,
    ) -> ActorRef<WeightcSessionActor> {
        while self.actors.len() >= max {
            self.evict_oldest().await;
        }
        let actor = WeightcSessionActor::spawn_link_with_mailbox(
            supervisor,
            WeightcSessionActor::new(key, spec),
            mailbox::bounded(mailbox_capacity),
        )
        .await;
        self.actors.insert(key.to_owned(), actor.clone());
        self.recency.push_back(key.to_owned());
        actor
    }

    /// Stop the least recently used actor; dropping its state kills the
    /// child via `kill_on_drop`.
    async fn evict_oldest(&mut self) {
        while let Some(oldest) = self.recency.pop_front() {
            if let Some(actor) = self.actors.remove(&oldest) {
                let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, actor.ask(StopSession)).await;
                let _ = actor.stop_gracefully().await;
                let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, actor.wait_for_shutdown()).await;
                return;
            }
        }
    }

    pub(super) fn remove(&mut self, key: &str) {
        self.actors.remove(key);
        self.recency.retain(|entry| entry != key);
    }

    fn touch(&mut self, key: &str) {
        self.recency.retain(|entry| entry != key);
        self.recency.push_back(key.to_owned());
    }

    pub(super) fn drain(&mut self) -> Vec<ActorRef<WeightcSessionActor>> {
        self.recency.clear();
        self.actors.drain().map(|(_, actor)| actor).collect()
    }
}

/// One user turn against a resident session.
pub(super) struct SessionTurn {
    pub(super) prompt: String,
    /// Written with the first successful turn only; later turns ignore it.
    pub(super) system: Option<String>,
}

/// Graceful actor request that kills and reaps the resident child before the
/// actor itself stops. `kill_on_drop` remains the abrupt-shutdown backstop.
pub(super) struct StopSession;

/// Turn result plus session liveness for the engine's session table.
#[derive(kameo::Reply)]
pub(super) struct TurnOutcome {
    pub(super) result: Result<Completion>,
    /// False when the session broke (child exited, timeout, protocol desync)
    /// and the engine must drop the actor.
    pub(super) alive: bool,
}

/// A failed turn: `alive` distinguishes weightc-reported error lines (the
/// session loop continues) from child/protocol breakage (the session is
/// torn down and dropped).
struct TurnFailure {
    error: LiveError,
    alive: bool,
}

impl TurnFailure {
    fn reported(error: LiveError) -> Self {
        Self { error, alive: true }
    }

    fn broken(error: LiveError) -> Self {
        Self {
            error,
            alive: false,
        }
    }
}

/// One resident `weightc enter --jsonl` child holding a conversation's KV
/// context. See module docs for the session lifecycle.
#[derive(Actor)]
pub(super) struct WeightcSessionActor {
    key: String,
    spec: SessionSpec,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    system_pending: Option<String>,
    greeted: bool,
}

impl WeightcSessionActor {
    fn new(key: &str, spec: SessionSpec) -> Self {
        Self {
            key: key.to_owned(),
            spec,
            child: None,
            stdin: None,
            stdout: None,
            system_pending: None,
            greeted: false,
        }
    }

    async fn ensure_child(&mut self) -> Result<(), TurnFailure> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut child = tokio::process::Command::new(&self.spec.binary)
            .arg("enter")
            .arg("--jsonl")
            .arg(&self.spec.artifact)
            .args(&self.spec.sampling)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                TurnFailure::broken(LiveError::Transport(format!(
                    "failed to start {}: {error}",
                    self.spec.binary
                )))
            })?;
        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        Ok(())
    }

    async fn teardown(&mut self) {
        self.stdin = None;
        self.stdout = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }

    async fn turn(&mut self, prompt: &str) -> Result<Completion, TurnFailure> {
        self.ensure_child().await?;
        let started = Instant::now();
        let system = if self.greeted {
            None
        } else {
            self.system_pending.as_deref()
        };
        let mut line = serde_json::to_string(&SessionRequestLine { prompt, system })
            .expect("session request line serializes");
        line.push('\n');
        self.write_line(&line).await?;
        self.read_receipt(started).await
    }

    async fn write_line(&mut self, line: &str) -> Result<(), TurnFailure> {
        let stdin = self.stdin.as_mut().expect("child stdin is piped");
        let result = match stdin.write_all(line.as_bytes()).await {
            Ok(()) => stdin.flush().await,
            Err(error) => Err(error),
        };
        result.map_err(|error| {
            TurnFailure::broken(LiveError::Transport(format!(
                "weightc session {} is gone (context lost): {error}",
                self.key
            )))
        })
    }

    async fn read_receipt(&mut self, started: Instant) -> Result<Completion, TurnFailure> {
        let mut line = String::new();
        loop {
            line.clear();
            let reader = self.stdout.as_mut().expect("child stdout is piped");
            match tokio::time::timeout(self.spec.timeout, reader.read_line(&mut line)).await {
                Err(_) => {
                    return Err(TurnFailure::broken(LiveError::Transport(format!(
                        "weightc session {} timed out after {}s",
                        self.key,
                        self.spec.timeout.as_secs()
                    ))));
                }
                Ok(Err(error)) => {
                    return Err(TurnFailure::broken(LiveError::Transport(format!(
                        "weightc session {} read failed (context lost): {error}",
                        self.key
                    ))));
                }
                Ok(Ok(0)) => {
                    return Err(TurnFailure::broken(LiveError::Transport(format!(
                        "weightc session {} exited (context lost)",
                        self.key
                    ))));
                }
                Ok(Ok(_)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    return self.parse_receipt(trimmed, started);
                }
            }
        }
    }

    fn parse_receipt(&mut self, line: &str, started: Instant) -> Result<Completion, TurnFailure> {
        let parsed: SessionLine = serde_json::from_str(line).map_err(|error| {
            TurnFailure::broken(LiveError::Protocol(format!(
                "weightc session {} emitted an unparsable line: {error}",
                self.key
            )))
        })?;
        if let Some(error) = parsed.error {
            return Err(TurnFailure::reported(match error.code.as_str() {
                "invalid_request" => {
                    LiveError::Protocol(format!("weightc rejected the turn: {}", error.message))
                }
                "session" => {
                    LiveError::Transport(format!("weightc session error: {}", error.message))
                }
                other => LiveError::Protocol(format!("weightc error {other}: {}", error.message)),
            }));
        }
        let text = parsed
            .response
            .or(parsed.text)
            .or(parsed.output)
            .ok_or_else(|| {
                TurnFailure::broken(LiveError::Protocol(format!(
                    "weightc session {} receipt has no response, text, or output field",
                    self.key
                )))
            })?;
        self.greeted = true;
        Ok(Completion {
            text,
            model: self.spec.model.clone(),
            tokens_per_second: parsed.tokens_per_second,
            elapsed_millis: parsed.elapsed_ms.unwrap_or_else(|| elapsed_millis(started)),
        })
    }
}

impl Message<SessionTurn> for WeightcSessionActor {
    type Reply = TurnOutcome;

    async fn handle(
        &mut self,
        message: SessionTurn,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if self.system_pending.is_none() {
            self.system_pending = message.system;
        }
        match self.turn(&message.prompt).await {
            Ok(completion) => TurnOutcome {
                result: Ok(completion),
                alive: true,
            },
            Err(failure) => {
                if !failure.alive {
                    self.teardown().await;
                }
                TurnOutcome {
                    result: Err(failure.error),
                    alive: failure.alive,
                }
            }
        }
    }
}

impl Message<StopSession> for WeightcSessionActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _message: StopSession,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.teardown().await;
    }
}

#[derive(Debug, Serialize)]
struct SessionRequestLine<'a> {
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
}

/// Tolerant receipt reader: unknown fields are ignored and a line carries
/// either a receipt or an `error` object.
#[derive(Debug, Deserialize)]
struct SessionLine {
    response: Option<String>,
    text: Option<String>,
    output: Option<String>,
    tokens_per_second: Option<f64>,
    elapsed_ms: Option<u64>,
    error: Option<SessionErrorLine>,
}

#[derive(Debug, Deserialize)]
struct SessionErrorLine {
    code: String,
    message: String,
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use crate::inference::weightc::WeightcEngine;
    use crate::inference::{CompletionRequest, InferenceEngine};
    use crate::models::ModelCatalog;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::Arc;

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

    /// Fake `weightc enter --jsonl`: logs start/turn lines (with pid) to
    /// `<artifact>/session.log`, answers canned receipts, emits an error
    /// line for prompts containing "boom", and exits on "die". Its `ask`
    /// mode proves the one-shot path.
    const SESSION_SCRIPT: &str = r#"#!/bin/sh
if [ "$1" = "ask" ]; then
  printf '{"response": "one-shot"}\n'
  exit 0
fi
artifact="$3"
log="$artifact/session.log"
printf 'start %s\n' "$$" >> "$log"
while IFS= read -r line; do
  [ -z "$line" ] && continue
  printf 'turn %s %s\n' "$$" "$line" >> "$log"
  case "$line" in
    *boom*) printf '{"error":{"code":"invalid_request","message":"bad turn"}}\n' ;;
    *die*) exit 1 ;;
    *)
      prompt=$(printf '%s' "$line" | sed -n 's/.*"prompt":"\([^"]*\)".*/\1/p')
      printf '{"response":"session:%s","generated_tokens":3,"elapsed_ms":7,"tokens_per_second":42.0,"finish_reason":"end_of_sequence"}\n' "$prompt"
      ;;
  esac
done
"#;

    fn resident_config(fixture: &WeightcFixture, max_sessions: usize) -> InferenceConfig {
        InferenceConfig {
            resident_sessions: true,
            max_resident_sessions: max_sessions,
            ..config_for(fixture)
        }
    }

    fn session_prompt(key: &str, text: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: text.to_owned(),
            session_key: Some(key.to_owned()),
            ..CompletionRequest::default()
        }
    }

    fn session_log(fixture: &WeightcFixture) -> String {
        let artifact = fixture
            .catalog
            .artifact_dir(&fixture.model_id)
            .expect("artifact dir");
        std::fs::read_to_string(artifact.join("session.log")).expect("session log")
    }

    fn log_lines<'a>(log: &'a str, prefix: &str) -> Vec<&'a str> {
        log.lines()
            .filter(|line| line.starts_with(prefix))
            .collect()
    }

    fn pid_alive(pid: &str) -> bool {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn wait_for_pid_death(pid: &str) -> bool {
        for _ in 0..30 {
            if !pid_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    fn started_pid(log: &str, index: usize) -> String {
        log_lines(log, "start ")[index]
            .trim_start_matches("start ")
            .to_owned()
    }

    #[tokio::test]
    async fn resident_sessions_reuse_one_child_and_shutdown_kills_it() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let engine = WeightcEngine::new(&resident_config(&fixture, 4), fixture.catalog.clone(), 8);
        assert!(engine.supports_sessions());

        let first = engine
            .complete(session_prompt("c1", "hi"))
            .await
            .expect("first turn");
        assert_eq!(first.text, "session:hi");
        assert_eq!(first.tokens_per_second, Some(42.0));
        assert_eq!(first.elapsed_millis, 7);
        let second = engine
            .complete(session_prompt("c1", "again"))
            .await
            .expect("second turn");
        assert_eq!(second.text, "session:again");

        let log = session_log(&fixture);
        assert_eq!(log_lines(&log, "start ").len(), 1, "log: {log}");
        assert_eq!(log_lines(&log, "turn ").len(), 2, "log: {log}");

        let pid = started_pid(&log, 0);
        engine.shutdown().await;
        assert!(wait_for_pid_death(&pid), "session child survived shutdown");
    }

    #[tokio::test]
    async fn session_actor_sends_system_with_the_first_successful_turn_only() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let actors = crate::actor::ActorSystem::start();
        let artifact = fixture
            .catalog
            .artifact_dir(&fixture.model_id)
            .expect("artifact dir");
        let spec = SessionSpec {
            binary: fixture.binary.to_string_lossy().into_owned(),
            artifact: artifact.clone(),
            sampling: Vec::new(),
            timeout: Duration::from_secs(30),
            model: fixture.model_id.clone(),
        };
        let actor = WeightcSessionActor::spawn_link_with_mailbox(
            actors.root(),
            WeightcSessionActor::new("conv-1", spec),
            mailbox::bounded(8),
        )
        .await;
        let turn = |prompt: &str| SessionTurn {
            prompt: prompt.to_owned(),
            system: Some("be kind".to_owned()),
        };
        let first = actor.ask(turn("hi")).await.expect("first turn");
        assert!(first.alive);
        first.result.expect("first receipt");
        let second = actor.ask(turn("again")).await.expect("second turn");
        second.result.expect("second receipt");

        let log = std::fs::read_to_string(artifact.join("session.log")).expect("session log");
        let turns = log_lines(&log, "turn ");
        assert_eq!(turns.len(), 2, "log: {log}");
        assert!(turns[0].contains("\"system\":\"be kind\""), "log: {log}");
        assert!(!turns[1].contains("system"), "log: {log}");

        let _ = actor.stop_gracefully().await;
        actor.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn error_lines_are_typed_and_the_session_survives() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let engine = WeightcEngine::new(&resident_config(&fixture, 4), fixture.catalog.clone(), 8);

        let error = engine
            .complete(session_prompt("c1", "boom"))
            .await
            .expect_err("error line must fail");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("bad turn"), "{error}");

        let ok = engine
            .complete(session_prompt("c1", "fine"))
            .await
            .expect("the session loop continues after error lines");
        assert_eq!(ok.text, "session:fine");
        let log = session_log(&fixture);
        assert_eq!(log_lines(&log, "start ").len(), 1, "log: {log}");
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn child_death_fails_the_turn_and_the_next_request_respawns() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let engine = WeightcEngine::new(&resident_config(&fixture, 4), fixture.catalog.clone(), 8);

        let error = engine
            .complete(session_prompt("c9", "die"))
            .await
            .expect_err("child death must fail the turn");
        assert_eq!(error.code(), "LIVE_TRANSPORT_UNAVAILABLE");
        assert!(error.to_string().contains("c9"), "{error}");

        let ok = engine
            .complete(session_prompt("c9", "fresh"))
            .await
            .expect("the next request lazily respawns a fresh session");
        assert_eq!(ok.text, "session:fresh");
        let log = session_log(&fixture);
        assert_eq!(log_lines(&log, "start ").len(), 2, "log: {log}");
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn lru_eviction_kills_the_evicted_child() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let engine = WeightcEngine::new(&resident_config(&fixture, 1), fixture.catalog.clone(), 8);

        engine
            .complete(session_prompt("old", "hi"))
            .await
            .expect("first session");
        let first_pid = started_pid(&session_log(&fixture), 0);
        engine
            .complete(session_prompt("new", "hi"))
            .await
            .expect("second session evicts the first");

        assert!(
            wait_for_pid_death(&first_pid),
            "evicted session child survived"
        );
        let log = session_log(&fixture);
        assert_eq!(log_lines(&log, "start ").len(), 2, "log: {log}");
        assert!(pid_alive(&started_pid(&log, 1)), "new session must live");
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resident_disabled_ignores_the_session_key() {
        let fixture = weightc_fixture(SESSION_SCRIPT);
        let engine = WeightcEngine::new(&config_for(&fixture), fixture.catalog.clone(), 8);
        assert!(!engine.supports_sessions());

        let completion = engine
            .complete(session_prompt("c1", "hi"))
            .await
            .expect("one-shot path");
        assert_eq!(completion.text, "one-shot");
        let artifact = fixture
            .catalog
            .artifact_dir(&fixture.model_id)
            .expect("artifact dir");
        assert!(!artifact.join("session.log").exists());
    }
}
