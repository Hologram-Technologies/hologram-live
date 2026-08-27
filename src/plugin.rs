//! Supervised subprocess plugin modules.
//!
//! Third-party modules run as separate operating-system processes speaking the
//! `hologram.live.plugin.v1` gRPC contract over a Unix domain socket. The
//! daemon spawns only executables from the explicit `[plugins]` allowlist and
//! re-verifies each executable's pinned sha256 before every spawn. The child
//! environment is scrubbed to a single variable, `HOLOGRAM_PLUGIN_SOCKET`,
//! carrying the socket path the plugin must serve on.
//!
//! Each allowlisted plugin is owned by one Kameo supervisor actor with a
//! bounded mailbox (sized by `server.actor_mailbox_capacity`). The supervisor
//! holds the child process handle and the tonic client, answers `Invoke`
//! messages, and restarts the child with capped backoff (three attempts) when
//! the transport fails mid-call. Health checking is deliberately minimal: a
//! `Ping` is part of the (re)connect handshake after `Describe`, so every
//! restart is verified before traffic resumes.
//!
//! Plugins are spawned and handshaken eagerly inside the async
//! `AppState::build`. A plugin that fails to start never blocks the daemon:
//! the failure is recorded in its `PluginStatus`, startup retries lazily on
//! the first invocation, and invocations of unknown or disabled plugins keep
//! the `LIVE_CAPABILITY_MISSING` semantics the desktop already recovers
//! around.
//!
//! v1 capability posture: plugins receive no host resource access — no store,
//! no config, no network mediation. They are pure compute over their JSON
//! input. ADR 005 documents the boundary and the mvm-based hardening path.
//!
//! Unix domain sockets are the only transport; on non-unix targets the
//! registry builds empty and every plugin operation degrades to
//! `LIVE_CAPABILITY_MISSING`.

use crate::actor::RootSupervisor;
use crate::config::{PluginModuleConfig, PluginsConfig};
use crate::error::{ApiError, LiveError, Result};
use crate::protocol::{ModuleInfo, OperationInfo, OperationKind, PluginStatus, PROTOCOL_VERSION};
use crate::util::{constant_time_eq, hex};
use kameo::actor::{ActorRef, Spawn};
use kameo::error::SendError;
use kameo::mailbox;
use kameo::message::{Context, Message};
use kameo::Actor;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tonic::transport::Channel;

#[allow(clippy::all, clippy::pedantic)]
pub mod pb {
    tonic::include_proto!("hologram.live.plugin.v1");
}

/// The only environment variable a plugin process receives.
pub const SOCKET_ENV: &str = "HOLOGRAM_PLUGIN_SOCKET";

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const INVOKE_TIMEOUT: Duration = Duration::from_mins(2);
const CONNECT_ATTEMPTS: u32 = 50;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_RESTARTS: u32 = 3;

type PluginClient = pb::plugin_host_client::PluginHostClient<Channel>;

/// One operation invocation against a plugin.
pub struct Invoke {
    pub operation: String,
    pub payload: Vec<u8>,
}

/// (Re)spawn the child, connect, and run the `Describe`/`Ping` handshake.
struct Start;

#[derive(Clone, kameo::Reply)]
struct SupervisorStatus {
    running: bool,
    restart_count: u64,
    last_error: Option<String>,
}

struct StatusQuery;

/// Kameo supervisor owning one plugin child process and its gRPC client.
#[derive(Actor)]
pub struct PluginSupervisor {
    spec: PluginModuleConfig,
    socket_path: PathBuf,
    client: Option<PluginClient>,
    child: Option<Child>,
    restart_count: u64,
    last_error: Option<String>,
}

impl PluginSupervisor {
    fn new(spec: PluginModuleConfig, socket_path: PathBuf) -> Self {
        Self {
            spec,
            socket_path,
            client: None,
            child: None,
            restart_count: 0,
            last_error: None,
        }
    }

    async fn start(&mut self) -> Result<pb::PluginDescriptor> {
        self.teardown().await;
        let result = self.start_verified().await;
        if let Err(error) = &result {
            self.last_error = Some(error.to_string());
            self.teardown().await;
        }
        result
    }

    async fn start_verified(&mut self) -> Result<pb::PluginDescriptor> {
        verify_sha256(&self.spec.path, &self.spec.sha256).await?;
        // The plugin binds the socket itself; drop any stale filesystem entry
        // left behind by a crashed child first.
        let _ = std::fs::remove_file(&self.socket_path);
        let child = tokio::process::Command::new(&self.spec.path)
            .env_clear()
            .env(SOCKET_ENV, &self.socket_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                LiveError::Io(format!(
                    "spawn plugin {} from {}: {error}",
                    self.spec.id,
                    self.spec.path.display()
                ))
            })?;
        self.child = Some(child);
        let channel = connect(&self.socket_path).await?;
        let mut client = PluginClient::new(channel);
        let descriptor = handshake(&mut client, &self.spec.id).await?;
        self.client = Some(client);
        Ok(descriptor)
    }

    async fn teardown(&mut self) {
        self.client = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Message<Start> for PluginSupervisor {
    type Reply = Result<pb::PluginDescriptor>;

    async fn handle(
        &mut self,
        _message: Start,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.start().await
    }
}

impl Message<Invoke> for PluginSupervisor {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        message: Invoke,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let mut restarts = 0_u32;
        loop {
            if self.client.is_none() {
                self.start().await?;
            }
            let client = self
                .client
                .as_mut()
                .expect("client is connected after start");
            let request = pb::InvokeRequest {
                operation: message.operation.clone(),
                payload: message.payload.clone(),
            };
            match client.invoke(request).await {
                Ok(response) => return invoke_result(response.into_inner()),
                Err(status) if restartable(&status) && restarts < MAX_RESTARTS => {
                    restarts += 1;
                    self.restart_count += 1;
                    self.last_error = Some(format!("transport failure: {}", status.message()));
                    self.teardown().await;
                    tokio::time::sleep(CONNECT_RETRY_DELAY * restarts).await;
                }
                Err(status) => {
                    let error = status_error(&status);
                    self.last_error = Some(error.to_string());
                    return Err(error);
                }
            }
        }
    }
}

impl Message<StatusQuery> for PluginSupervisor {
    type Reply = SupervisorStatus;

    async fn handle(
        &mut self,
        _message: StatusQuery,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        SupervisorStatus {
            running: self.client.is_some(),
            restart_count: self.restart_count,
            last_error: self.last_error.clone(),
        }
    }
}

/// The daemon-side registry of allowlisted plugins, held on `AppState`
/// alongside the builtin `ModuleRegistry`.
pub struct PluginRegistry {
    entries: Vec<PluginEntry>,
    /// Plugin-provided operation ids merged into the capability manifest.
    operations: BTreeMap<String, OperationKind>,
}

struct PluginEntry {
    id: String,
    name: String,
    version: String,
    operations: Vec<(String, OperationKind)>,
    actor: ActorRef<PluginSupervisor>,
    start_error: Option<String>,
}

impl PluginRegistry {
    /// Spawn and handshake every allowlisted plugin. Per-plugin failures are
    /// recorded as status, never fatal to the daemon; an empty or disabled
    /// allowlist is a no-op.
    pub async fn build(
        config: &PluginsConfig,
        state_dir: &Path,
        mailbox_capacity: usize,
        supervisor: &ActorRef<RootSupervisor>,
    ) -> Result<Self> {
        let mut registry = Self {
            entries: Vec::new(),
            operations: BTreeMap::new(),
        };
        if !config.enabled || config.modules.is_empty() {
            return Ok(registry);
        }
        let socket_dir = state_dir.join("plugins");
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .map_err(|error| LiveError::io(&socket_dir, error))?;
        for spec in &config.modules {
            registry
                .add(spec, &socket_dir, mailbox_capacity, supervisor)
                .await;
        }
        Ok(registry)
    }

    async fn add(
        &mut self,
        spec: &PluginModuleConfig,
        socket_dir: &Path,
        mailbox_capacity: usize,
        supervisor: &ActorRef<RootSupervisor>,
    ) {
        let socket_path = socket_dir.join(format!("{}.sock", socket_name(&spec.id)));
        let actor = PluginSupervisor::spawn_link_with_mailbox(
            supervisor,
            PluginSupervisor::new(spec.clone(), socket_path),
            mailbox::bounded(mailbox_capacity.max(1)),
        )
        .await;
        let started = match actor.ask(Start).await {
            Ok(descriptor) => Ok(descriptor),
            Err(SendError::HandlerError(error)) => Err(error),
            Err(error) => Err(LiveError::Conflict(format!(
                "plugin {} supervisor mailbox failed: {error}",
                spec.id
            ))),
        };
        match started {
            Ok(descriptor) => {
                for operation in &descriptor.operations {
                    if self
                        .operations
                        .insert(operation.id.clone(), operation_kind(operation.kind))
                        .is_some()
                    {
                        tracing::warn!(
                            operation = %operation.id,
                            plugin = %spec.id,
                            "plugin operation shadows an earlier plugin operation"
                        );
                    }
                }
                self.entries.push(PluginEntry {
                    id: spec.id.clone(),
                    name: descriptor.name,
                    version: descriptor.version,
                    operations: descriptor
                        .operations
                        .iter()
                        .map(|operation| (operation.id.clone(), operation_kind(operation.kind)))
                        .collect(),
                    actor,
                    start_error: None,
                });
            }
            Err(error) => {
                tracing::warn!(
                    plugin = %spec.id,
                    error = %error,
                    "plugin failed to start; it retries lazily on first invocation"
                );
                self.entries.push(PluginEntry {
                    id: spec.id.clone(),
                    name: String::new(),
                    version: String::new(),
                    operations: Vec::new(),
                    actor,
                    start_error: Some(error.to_string()),
                });
            }
        }
    }

    /// Whether any started plugin provides `operation`. Consulted by
    /// `AppState::dispatch` after the builtin registry misses.
    pub fn supports(&self, operation: &str) -> bool {
        self.operations.contains_key(operation)
    }

    /// Plugin-provided operations for the capability manifest.
    pub fn operations(&self) -> Vec<OperationInfo> {
        self.operations
            .iter()
            .map(|(id, kind)| OperationInfo {
                id: id.clone(),
                kind: *kind,
                fallback_safe_before_dispatch: *kind == OperationKind::Read,
            })
            .collect()
    }

    /// Plugin descriptors in `ModuleInfo` shape for the capability manifest.
    pub fn info(&self) -> Vec<ModuleInfo> {
        self.entries
            .iter()
            .map(|entry| ModuleInfo {
                id: entry.id.clone(),
                name: entry.name.clone(),
                version: entry.version.clone(),
                state: if entry.start_error.is_some() {
                    "failed".to_owned()
                } else {
                    "ready".to_owned()
                },
                dependencies: Vec::new(),
                operations: entry.operations.iter().map(|(id, _)| id.clone()).collect(),
            })
            .collect()
    }

    pub async fn list(&self) -> Vec<PluginStatus> {
        let mut statuses = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let runtime = entry.actor.ask(StatusQuery).await.ok();
            statuses.push(PluginStatus {
                id: entry.id.clone(),
                name: entry.name.clone(),
                version: entry.version.clone(),
                operations: entry.operations.iter().map(|(id, _)| id.clone()).collect(),
                running: runtime.as_ref().is_some_and(|status| status.running),
                restart_count: runtime.as_ref().map_or(0, |status| status.restart_count),
                last_error: runtime
                    .and_then(|status| status.last_error)
                    .or_else(|| entry.start_error.clone()),
            });
        }
        statuses
    }

    /// Forward a JSON payload to a plugin operation. Unknown plugins and
    /// undeclared operations keep `LIVE_CAPABILITY_MISSING` semantics.
    pub async fn invoke(&self, plugin_id: &str, operation: &str, payload: &str) -> Result<String> {
        if let Err(error) = serde_json::from_str::<serde_json::Value>(payload) {
            return Err(LiveError::Protocol(format!(
                "plugin.call payload must be valid JSON: {error}"
            )));
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == plugin_id)
            .ok_or_else(|| {
                LiveError::Capability(format!("plugin {plugin_id} is not enabled on this server"))
            })?;
        if !entry.operations.is_empty() && !entry.operations.iter().any(|(id, _)| id == operation) {
            return Err(LiveError::Capability(format!(
                "plugin {plugin_id} does not provide operation {operation}"
            )));
        }
        let ask = entry
            .actor
            .ask(Invoke {
                operation: operation.to_owned(),
                payload: payload.as_bytes().to_vec(),
            })
            .await;
        match ask {
            Ok(reply) => Ok(reply),
            Err(SendError::HandlerError(error)) => Err(error),
            Err(error) => Err(LiveError::Transport(format!(
                "plugin {plugin_id} supervisor is unavailable: {error}"
            ))),
        }
    }

    /// Stop every supervisor actor, which kills its child process. The
    /// `kill_on_drop` child handle is the backstop for abrupt daemon exits.
    pub async fn shutdown(&self) {
        for entry in &self.entries {
            let _ = entry.actor.stop_gracefully().await;
        }
        for entry in &self.entries {
            let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, entry.actor.wait_for_shutdown()).await;
        }
    }
}

/// Map the wire kind to the native kind; unknown or unspecified kinds are
/// treated as mutations so they keep the stricter audit path.
fn operation_kind(kind: i32) -> OperationKind {
    match pb::PluginOperationKind::try_from(kind) {
        Ok(pb::PluginOperationKind::Read) => OperationKind::Read,
        Ok(pb::PluginOperationKind::Stream) => OperationKind::Stream,
        Ok(pb::PluginOperationKind::Mutation | pb::PluginOperationKind::Unspecified) | Err(_) => {
            OperationKind::Mutation
        }
    }
}

/// Bounded, filesystem-safe socket file name: `blake3(id)[..16]`.
fn socket_name(id: &str) -> String {
    let digest = blake3::hash(id.as_bytes()).to_hex().to_string();
    digest[..16].to_owned()
}

async fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| LiveError::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| LiveError::io(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hex(&hasher.finalize());
    if !constant_time_eq(actual.as_bytes(), expected.to_ascii_lowercase().as_bytes()) {
        return Err(LiveError::Authorization(format!(
            "sha256 mismatch for plugin executable {} (expected {expected}, got {actual}); \
             refusing to spawn",
            path.display()
        )));
    }
    Ok(())
}

/// Connect to the plugin socket, retrying while the freshly spawned child
/// binds it. UDS-only: non-unix targets degrade to a capability error.
#[cfg(unix)]
async fn connect(socket_path: &Path) -> Result<Channel> {
    let endpoint = tonic::transport::Endpoint::from_shared("http://[::]:0")
        .map_err(|error| LiveError::Protocol(format!("plugin endpoint: {error}")))?
        .timeout(INVOKE_TIMEOUT)
        .connect_timeout(HANDSHAKE_TIMEOUT);
    let mut last_error = String::new();
    for _ in 0..CONNECT_ATTEMPTS {
        let socket = socket_path.to_path_buf();
        let connector = tower::service_fn(move |_: tonic::transport::Uri| {
            let socket = socket.clone();
            async move {
                tokio::net::UnixStream::connect(socket)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        });
        match endpoint.connect_with_connector(connector).await {
            Ok(channel) => return Ok(channel),
            Err(error) => {
                last_error = error.to_string();
                tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            }
        }
    }
    Err(LiveError::Transport(format!(
        "plugin did not listen on {} within {} attempts: {last_error}",
        socket_path.display(),
        CONNECT_ATTEMPTS
    )))
}

#[cfg(not(unix))]
async fn connect(_socket_path: &Path) -> Result<Channel> {
    Err(LiveError::Capability(
        "subprocess plugins require unix domain sockets; unsupported on this platform".to_owned(),
    ))
}

/// `Describe` handshake with id and protocol checks, then `Ping` as the
/// on-connect health check.
async fn handshake(client: &mut PluginClient, expected_id: &str) -> Result<pb::PluginDescriptor> {
    let descriptor =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, client.describe(pb::DescribeRequest {}))
            .await
            .map_err(|_| LiveError::Transport(format!("plugin {expected_id} describe timed out")))?
            .map_err(|status| {
                LiveError::Transport(format!(
                    "plugin {expected_id} describe failed: {}",
                    status.message()
                ))
            })?
            .into_inner();
    if descriptor.id != expected_id {
        return Err(LiveError::Protocol(format!(
            "plugin handshook as {} but the allowlist expects {expected_id}",
            descriptor.id
        )));
    }
    if descriptor.min_protocol > u32::from(PROTOCOL_VERSION) {
        return Err(LiveError::Capability(format!(
            "plugin {expected_id} requires protocol v{} but this server speaks v{PROTOCOL_VERSION}",
            descriptor.min_protocol
        )));
    }
    tokio::time::timeout(HANDSHAKE_TIMEOUT, client.ping(pb::PingRequest {}))
        .await
        .map_err(|_| LiveError::Transport(format!("plugin {expected_id} ping timed out")))?
        .map_err(|status| {
            LiveError::Transport(format!(
                "plugin {expected_id} ping failed: {}",
                status.message()
            ))
        })?;
    Ok(descriptor)
}

fn invoke_result(response: pb::InvokeResponse) -> Result<String> {
    if response.error_code.is_empty() {
        return String::from_utf8(response.result)
            .map_err(|_| LiveError::Protocol("plugin returned a non-UTF-8 result".to_owned()));
    }
    Err(LiveError::from(ApiError {
        code: response.error_code,
        message: response.error_message,
    }))
}

/// Only connection-level failures justify a child restart; application-level
/// status codes are returned to the caller unchanged.
fn restartable(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unavailable | tonic::Code::Unknown
    )
}

fn status_error(status: &tonic::Status) -> LiveError {
    let message = status.message().to_owned();
    match status.code() {
        tonic::Code::Unimplemented => LiveError::Capability(message),
        tonic::Code::NotFound => LiveError::NotFound(message),
        tonic::Code::InvalidArgument => LiveError::Protocol(message),
        _ => LiveError::Transport(format!("plugin transport failure: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorSystem;

    /// Kills the child process while leaving the (now dead) client in place,
    /// simulating a plugin crash between invocations.
    struct KillChild;

    impl Message<KillChild> for PluginSupervisor {
        type Reply = ();

        async fn handle(
            &mut self,
            _message: KillChild,
            _context: &mut Context<Self, Self::Reply>,
        ) -> Self::Reply {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
    }

    #[tokio::test]
    async fn spawn_handshake_invoke_and_bounded_restart() {
        let directory = test_dir("sup");
        let actors = ActorSystem::start();
        let socket_path = directory.join("plugins").join("echo.sock");
        std::fs::create_dir_all(socket_path.parent().expect("socket parent"))
            .expect("socket directory");
        let actor = PluginSupervisor::spawn_link_with_mailbox(
            actors.root(),
            PluginSupervisor::new(example_spec(), socket_path),
            mailbox::bounded(8),
        )
        .await;

        let descriptor = actor.ask(Start).await.expect("handshake");
        assert_eq!(descriptor.id, "dev.hologram.examples.echo");
        assert_eq!(descriptor.operations.len(), 1);
        assert_eq!(descriptor.operations[0].id, "echo.ping");

        let echoed = invoke(&actor, r#"{"hi":1}"#).await;
        assert_eq!(echoed, r#"{"echo":{"hi":1}}"#);

        actor.ask(KillChild).await.expect("kill child");
        let echoed = invoke(&actor, r#"{"again":true}"#).await;
        assert_eq!(echoed, r#"{"echo":{"again":true}}"#);
        let status = actor.ask(StatusQuery).await.expect("status");
        assert!(status.running);
        assert_eq!(status.restart_count, 1);

        let _ = actor.stop_gracefully().await;
        actor.wait_for_shutdown().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn sha256_mismatch_refuses_to_spawn() {
        let directory = test_dir("sha");
        let actors = ActorSystem::start();
        let mut spec = example_spec();
        spec.sha256 = "00".repeat(32);
        let actor = PluginSupervisor::spawn_link_with_mailbox(
            actors.root(),
            PluginSupervisor::new(spec, directory.join("plugins/echo.sock")),
            mailbox::bounded(8),
        )
        .await;
        let error = match actor.ask(Start).await {
            Err(SendError::HandlerError(error)) => error,
            other => panic!("expected a handler error, got {other:?}"),
        };
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        let _ = actor.stop_gracefully().await;
        actor.wait_for_shutdown().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn registry_invokes_lists_and_preserves_capability_semantics() {
        let directory = test_dir("reg");
        let actors = ActorSystem::start();
        let config = PluginsConfig {
            enabled: true,
            modules: vec![example_spec()],
        };
        let registry = PluginRegistry::build(&config, &directory, 8, actors.root())
            .await
            .expect("build registry");

        assert!(registry.supports("echo.ping"));
        assert_eq!(registry.operations().len(), 1);
        let info = registry.info();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].state, "ready");

        let list = registry.list().await;
        assert_eq!(list.len(), 1);
        assert!(list[0].running);
        assert_eq!(list[0].restart_count, 0);
        assert_eq!(list[0].operations, vec!["echo.ping".to_owned()]);

        let result = registry
            .invoke("dev.hologram.examples.echo", "echo.ping", r#"{"a":2}"#)
            .await
            .expect("invoke");
        assert_eq!(result, r#"{"echo":{"a":2}}"#);

        let error = registry
            .invoke("dev.hologram.examples.missing", "echo.ping", "{}")
            .await
            .expect_err("unknown plugin must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");

        let error = registry
            .invoke("dev.hologram.examples.echo", "echo.nope", "{}")
            .await
            .expect_err("unknown operation must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");

        let error = registry
            .invoke("dev.hologram.examples.echo", "echo.ping", "not json")
            .await
            .expect_err("invalid JSON must fail");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");

        registry.shutdown().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn disabled_plugins_are_a_noop() {
        let directory = test_dir("off");
        let actors = ActorSystem::start();
        let config = PluginsConfig::default();
        let registry = PluginRegistry::build(&config, &directory, 8, actors.root())
            .await
            .expect("build registry");
        assert!(!registry.supports("echo.ping"));
        assert!(registry.operations().is_empty());
        assert!(registry.list().await.is_empty());
        let error = registry
            .invoke("dev.hologram.examples.echo", "echo.ping", "{}")
            .await
            .expect_err("disabled plugin must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        let _ = std::fs::remove_dir_all(directory);
    }

    async fn invoke(actor: &ActorRef<PluginSupervisor>, payload: &str) -> String {
        actor
            .ask(Invoke {
                operation: "echo.ping".to_owned(),
                payload: payload.as_bytes().to_vec(),
            })
            .await
            .expect("invoke echo.ping")
    }

    fn example_spec() -> PluginModuleConfig {
        let path = example_plugin_path();
        PluginModuleConfig {
            id: "dev.hologram.examples.echo".to_owned(),
            sha256: hex(&Sha256::digest(
                std::fs::read(&path).expect("read plugin binary"),
            )),
            path,
        }
    }

    /// Unit tests drive the dev-only example binary; build it on demand so a
    /// bare `cargo test` run works without a prior `cargo build --examples`.
    fn example_plugin_path() -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target =
            std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
        let binary = target.join("debug/examples/echo-plugin");
        if !binary.exists() {
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--locked", "--example", "echo-plugin"])
                .current_dir(root)
                .status()
                .expect("build the echo-plugin example");
            assert!(status.success(), "building the echo-plugin example failed");
        }
        binary
    }

    /// UDS paths are length-limited (104 bytes on macOS), so keep test state
    /// directories short instead of deriving them from `tempfile`.
    fn test_dir(name: &str) -> PathBuf {
        let directory = PathBuf::from(format!("/tmp/hplg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create test directory");
        directory
    }
}
