use crate::error::{LiveError, Result};
use crate::util::{atomic_write, expand_home, home_dir};
use serde::{Deserialize, Serialize};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const CURRENT_SCHEMA_VERSION: u32 = 2;
/// Oldest `schema_version` this build can read and upgrade in place.
/// Anything older is refused rather than guessed at.
const MINIMUM_SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// A key removed from the schema, and the `schema_version` that removed it.
#[derive(Debug, Clone, Copy)]
struct RetiredKey {
    /// Dotted path to the key, such as `server.max_rpc_bytes`. A path with no
    /// dot names a whole top-level table.
    path: &'static str,
    /// First `schema_version` that no longer accepts the key.
    removed_in: u32,
}

/// Keys this build no longer accepts.
///
/// `deny_unknown_fields` is kept deliberately, so a file still carrying a
/// removed key would otherwise be refused outright. Removing a field therefore
/// means bumping `CURRENT_SCHEMA_VERSION` and listing the field here: files
/// written before the bump keep starting, and the key is dropped as part of the
/// upgrade. An entry can go once `MINIMUM_SUPPORTED_SCHEMA_VERSION` has passed
/// its `removed_in`, because no readable file can contain it any more.
const RETIRED_KEYS: &[RetiredKey] = &[];

/// Drops retired keys from a parsed configuration, returning the paths removed.
///
/// Only keys retired *after* `from` are considered. A file already at the
/// current version is left untouched, so an unrecognised key there stays a hard
/// error rather than being silently discarded as a typo.
fn prune_retired_keys(table: &mut toml::Table, from: u32, retired: &[RetiredKey]) -> Vec<String> {
    let mut dropped = Vec::new();
    for key in retired.iter().filter(|key| key.removed_in > from) {
        if remove_dotted(table, key.path) {
            dropped.push(key.path.to_owned());
        }
    }
    dropped
}

/// Removes a dotted path, reporting whether anything was there.
fn remove_dotted(table: &mut toml::Table, path: &str) -> bool {
    match path.split_once('.') {
        None => table.remove(path).is_some(),
        Some((head, rest)) => table
            .get_mut(head)
            .and_then(toml::Value::as_table_mut)
            .is_some_and(|inner| remove_dotted(inner, rest)),
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
    #[default]
    Node,
    ControlPlane,
}

impl ServerRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::ControlPlane => "control_plane",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetPreference {
    Local,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub role: ServerRole,
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub client: ClientConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    #[serde(default)]
    pub modules: ModulesConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub inference: InferenceConfig,
    #[serde(default)]
    pub holo: HoloConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathsConfig {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: String,
    pub max_rpc_bytes: usize,
    pub max_http_body_bytes: usize,
    pub graceful_shutdown_secs: u64,
    pub actor_mailbox_capacity: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientConfig {
    pub preference: TargetPreference,
    pub local_endpoint: String,
    pub remote_endpoint: Option<String>,
    pub allow_read_fallback: bool,
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    pub required: bool,
    pub token_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TracingConfig {
    pub filter: String,
    pub format: String,
    pub include_target: bool,
    pub include_thread_ids: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryConfig {
    /// Enables creation of application metrics and trace context.
    pub enabled: bool,
    /// OTLP/gRPC collector endpoint. No data leaves the process when unset.
    pub endpoint: Option<String>,
    pub service_name: String,
    pub export_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModulesConfig {
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    pub manifest_url: Option<String>,
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InferenceConfig {
    /// echo | weightc | ollama
    pub engine: String,
    /// `blake3:...` of an imported model (weightc) or a model tag (ollama).
    pub default_model: String,
    pub weightc_path: String,
    pub ollama_endpoint: String,
    pub request_timeout_secs: u64,
    /// Keep `weightc enter --jsonl` sessions resident per conversation.
    /// Only meaningful for the weightc engine.
    pub resident_sessions: bool,
    /// Maximum concurrently resident weightc sessions; the least recently
    /// used session is evicted beyond this bound.
    pub max_resident_sessions: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HoloConfig {
    /// Explicit development-only effective grant for resident applications.
    /// Relative paths resolve from `paths.config_dir`.
    pub development_grant: Option<PathBuf>,
    /// Archives the service loads into resident sessions during startup,
    /// so declared applications are invocable immediately after boot or
    /// restart without an explicit `holo load`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resident: Vec<HoloResidentConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoloResidentConfig {
    /// Content κ (`blake3:` + 64 hex) of an archive already imported into
    /// the local catalog. Load failures are logged and skip only the
    /// failing entry; they do not stop the service from starting.
    pub kappa: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    /// Master switch for subprocess plugin modules. Defaults to off.
    pub enabled: bool,
    /// Explicit allowlist of plugin executables. There is no directory
    /// scanning: only entries listed here with a pinned sha256 are spawned.
    pub modules: Vec<PluginModuleConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginModuleConfig {
    /// Stable module id the plugin must declare in its `Describe` handshake.
    /// Must not collide with a builtin module id.
    pub id: String,
    /// Executable spawned by the daemon. No shell is involved.
    pub path: PathBuf,
    /// Lowercase hex sha256 of the executable, verified before every spawn.
    pub sha256: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            role: ServerRole::Node,
            paths: PathsConfig::default(),
            server: ServerConfig::default(),
            client: ClientConfig::default(),
            auth: AuthConfig::default(),
            tracing: TracingConfig::default(),
            telemetry: TelemetryConfig::default(),
            modules: ModulesConfig::default(),
            update: UpdateConfig::default(),
            inference: InferenceConfig::default(),
            holo: HoloConfig::default(),
            plugins: PluginsConfig::default(),
        }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        let home = home_dir();
        Self {
            config_dir: home.join(".config/hologram"),
            data_dir: home.join(".local/share/hologram"),
            state_dir: home.join(".local/state/hologram"),
            cache_dir: home.join(".cache/hologram"),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:11435".to_owned(),
            max_rpc_bytes: 32 * 1024 * 1024,
            max_http_body_bytes: 32 * 1024 * 1024,
            graceful_shutdown_secs: 30,
            actor_mailbox_capacity: 128,
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            preference: TargetPreference::Local,
            local_endpoint: "http://127.0.0.1:11435".to_owned(),
            remote_endpoint: None,
            allow_read_fallback: true,
            request_timeout_secs: 30,
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            required: false,
            token_env: "HOLOGRAM_AUTH_TOKEN".to_owned(),
        }
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            filter: "info,hologram_live=debug".to_owned(),
            format: "pretty".to_owned(),
            include_target: true,
            include_thread_ids: false,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: None,
            service_name: "hologram-live".to_owned(),
            export_timeout_secs: 5,
        }
    }
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            enabled: crate::modules::default_builtin_ids(),
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            manifest_url: None,
            channel: "stable".to_owned(),
        }
    }
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            engine: "echo".to_owned(),
            default_model: String::new(),
            weightc_path: "weightc".to_owned(),
            ollama_endpoint: "http://127.0.0.1:11434".to_owned(),
            request_timeout_secs: 300,
            resident_sessions: false,
            max_resident_sessions: 4,
        }
    }
}

impl AppConfig {
    pub fn default_path() -> PathBuf {
        env::var_os("HOLOGRAM_CONFIG_DIR")
            .map_or_else(|| home_dir().join(".config/hologram"), PathBuf::from)
            .join("live.toml")
    }

    pub fn initialize(path: Option<&Path>, force: bool) -> Result<PathBuf> {
        let path = path.map_or_else(Self::default_path, expand_home);
        if path.exists() && !force {
            return Err(LiveError::Conflict(format!(
                "{} already exists; pass --force to replace it",
                path.display()
            )));
        }
        let config = Self::default();
        config.create_directories()?;
        let bytes = toml::to_string_pretty(&config)?;
        atomic_write(&path, bytes.as_bytes())?;
        Ok(path)
    }

    /// Loads the configuration, upgrading an older `schema_version` in place.
    pub fn load(path: Option<&Path>) -> Result<(Self, PathBuf)> {
        Self::load_inner(path, true)
    }

    /// Loads the configuration without upgrading it.
    ///
    /// Tracing is configured *from* the configuration, so the first read
    /// happens before a subscriber exists. Upgrading there would rewrite the
    /// file and drop the log line describing it. This read leaves the file
    /// alone and lets the first post-subscriber `load` perform and report the
    /// upgrade; an out-of-date file simply fails `validate` here, and the
    /// caller falls back to defaults for tracing.
    pub fn load_for_bootstrap(path: Option<&Path>) -> Result<(Self, PathBuf)> {
        Self::load_inner(path, false)
    }

    fn load_inner(path: Option<&Path>, upgrade: bool) -> Result<(Self, PathBuf)> {
        let path = path.map_or_else(Self::default_path, expand_home);
        let exists = path.exists();
        let mut config = if exists {
            let source =
                std::fs::read_to_string(&path).map_err(|error| LiveError::io(&path, error))?;
            Self::from_source(&source, upgrade, &path, RETIRED_KEYS)?
        } else {
            Self::default()
        };
        if upgrade {
            let from = config.schema_version;
            // Persist before environment overrides and `~` expansion are
            // applied, so only what the user actually wrote goes back to disk.
            if config.migrate()? && exists {
                config.persist_upgrade(&path, from);
            }
        }
        config.apply_environment()?;
        config.expand_paths();
        config.validate()?;
        Ok((config, path))
    }

    /// Parses a configuration file, dropping keys retired by a newer schema.
    ///
    /// The prune has to happen on the raw table: `deny_unknown_fields` rejects a
    /// retired key during deserialisation, before `schema_version` is ever read,
    /// so the upgrade path in `migrate` would never be reached.
    fn from_source(
        source: &str,
        upgrade: bool,
        path: &Path,
        retired: &[RetiredKey],
    ) -> Result<Self> {
        if !upgrade {
            return Ok(toml::from_str::<Self>(source)?);
        }
        let mut table = toml::from_str::<toml::Table>(source)?;
        // A file with no readable version is left alone; deserialisation below
        // reports the missing or malformed field.
        if let Some(from) = table
            .get("schema_version")
            .and_then(toml::Value::as_integer)
            .and_then(|version| u32::try_from(version).ok())
        {
            let dropped = prune_retired_keys(&mut table, from, retired);
            if !dropped.is_empty() {
                tracing::warn!(
                    config.path = %path.display(),
                    config.schema_from = from,
                    config.dropped_keys = %dropped.join(", "),
                    "dropped configuration keys retired by a newer schema"
                );
            }
        }
        Ok(toml::Value::Table(table).try_into::<Self>()?)
    }

    /// Brings a parsed configuration up to `CURRENT_SCHEMA_VERSION`, reporting
    /// whether anything changed.
    ///
    /// Schema growth has been additive, and missing fields already fall back to
    /// their defaults during deserialisation, so an upgrade is a version
    /// restamp rather than a field-by-field transform. Should a future version
    /// need to *move* or reinterpret a value, this is where that step belongs.
    fn migrate(&mut self) -> Result<bool> {
        if self.schema_version == CURRENT_SCHEMA_VERSION {
            return Ok(false);
        }
        // A newer file may carry settings this build would drop on write, so
        // refuse rather than silently downgrade it.
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(LiveError::Config(format!(
                "configuration schema {} is newer than this build supports ({CURRENT_SCHEMA_VERSION}); upgrade hologram to read it",
                self.schema_version
            )));
        }
        if self.schema_version < MINIMUM_SUPPORTED_SCHEMA_VERSION {
            return Err(LiveError::Config(format!(
                "configuration schema {} predates the oldest readable schema ({MINIMUM_SUPPORTED_SCHEMA_VERSION}); regenerate it with `hologram init --force`",
                self.schema_version
            )));
        }
        self.schema_version = CURRENT_SCHEMA_VERSION;
        Ok(true)
    }

    /// Writes an upgraded configuration back so the next start reads a current
    /// file.
    fn persist_upgrade(&self, path: &Path, from: u32) {
        let written = toml::to_string_pretty(self)
            .map_err(LiveError::from)
            .and_then(|encoded| atomic_write(path, encoded.as_bytes()));
        match written {
            Ok(()) => tracing::info!(
                config.path = %path.display(),
                config.schema_from = from,
                config.schema_to = CURRENT_SCHEMA_VERSION,
                "upgraded configuration schema"
            ),
            // An unwritable config directory must not stop the daemon: the
            // configuration in hand is already current, only the file lags.
            Err(error) => tracing::warn!(
                config.path = %path.display(),
                config.schema_from = from,
                error = %error,
                "configuration upgraded in memory but could not be written back"
            ),
        }
    }

    pub fn create_directories(&self) -> Result<()> {
        for path in [
            &self.paths.config_dir,
            &self.paths.data_dir,
            &self.paths.state_dir,
            &self.paths.cache_dir,
        ] {
            std::fs::create_dir_all(path).map_err(|error| LiveError::io(path, error))?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(LiveError::Config(format!(
                "unsupported configuration schema {}",
                self.schema_version
            )));
        }
        let listen: SocketAddr = self
            .server
            .listen
            .parse()
            .map_err(|error| LiveError::Config(format!("invalid server.listen: {error}")))?;
        if !is_loopback(listen.ip()) && !self.auth.required {
            return Err(LiveError::Config(
                "non-loopback server.listen requires auth.required = true".to_owned(),
            ));
        }
        if self.holo.development_grant.is_some() && !is_loopback(listen.ip()) {
            return Err(LiveError::Config(
                "holo.development_grant is development-only and requires a loopback server.listen"
                    .to_owned(),
            ));
        }
        if self.server.max_rpc_bytes == 0 || self.server.max_http_body_bytes == 0 {
            return Err(LiveError::Config(
                "message and body limits must be greater than zero".to_owned(),
            ));
        }
        if self.server.actor_mailbox_capacity == 0 {
            return Err(LiveError::Config(
                "server.actor_mailbox_capacity must be greater than zero".to_owned(),
            ));
        }
        if self.auth.required && env::var_os(&self.auth.token_env).is_none() {
            return Err(LiveError::Config(format!(
                "auth.required is true but {} is not set",
                self.auth.token_env
            )));
        }
        validate_endpoint(&self.client.local_endpoint, true)?;
        if let Some(remote) = &self.client.remote_endpoint {
            validate_endpoint(remote, false)?;
        }
        match self.tracing.format.as_str() {
            "pretty" | "compact" | "json" => {}
            other => {
                return Err(LiveError::Config(format!(
                    "unsupported tracing.format {other:?}"
                )))
            }
        }
        if let Some(endpoint) = &self.telemetry.endpoint {
            validate_endpoint(endpoint, false)?;
        }
        match self.inference.engine.as_str() {
            "echo" | "weightc" | "ollama" => {}
            other => {
                return Err(LiveError::Config(format!(
                    "unsupported inference.engine {other:?}; expected echo, weightc, or ollama"
                )))
            }
        }
        if self.inference.weightc_path.trim().is_empty() {
            return Err(LiveError::Config(
                "inference.weightc_path must not be empty".to_owned(),
            ));
        }
        validate_endpoint(&self.inference.ollama_endpoint, false)?;
        if self.inference.request_timeout_secs == 0 {
            return Err(LiveError::Config(
                "inference.request_timeout_secs must be greater than zero".to_owned(),
            ));
        }
        if self.inference.max_resident_sessions == 0 {
            return Err(LiveError::Config(
                "inference.max_resident_sessions must be greater than zero".to_owned(),
            ));
        }
        if self.telemetry.service_name.trim().is_empty() {
            return Err(LiveError::Config(
                "telemetry.service_name must not be empty".to_owned(),
            ));
        }
        if self.telemetry.export_timeout_secs == 0 {
            return Err(LiveError::Config(
                "telemetry.export_timeout_secs must be greater than zero".to_owned(),
            ));
        }
        self.validate_holo_resident()?;
        self.validate_plugins()?;
        Ok(())
    }

    fn validate_holo_resident(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.holo.resident {
            let kappa = entry.kappa.trim();
            let digest = kappa.strip_prefix("blake3:").ok_or_else(|| {
                LiveError::Config(format!(
                    "holo.resident kappa {kappa:?} must start with \"blake3:\""
                ))
            })?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(LiveError::Config(format!(
                    "holo.resident kappa {kappa:?} must be \"blake3:\" followed by 64 hex characters"
                )));
            }
            if !seen.insert(kappa.to_owned()) {
                return Err(LiveError::Config(format!(
                    "duplicate holo.resident kappa {kappa}"
                )));
            }
        }
        Ok(())
    }

    fn validate_plugins(&self) -> Result<()> {
        let builtin_ids = crate::modules::default_builtin_ids();
        let mut seen = std::collections::BTreeSet::new();
        for module in &self.plugins.modules {
            if module.id.trim().is_empty() {
                return Err(LiveError::Config(
                    "plugins.modules entries must declare a non-empty id".to_owned(),
                ));
            }
            if builtin_ids.iter().any(|id| id == &module.id) {
                return Err(LiveError::Config(format!(
                    "plugin id {} collides with a builtin module id",
                    module.id
                )));
            }
            if !seen.insert(module.id.clone()) {
                return Err(LiveError::Config(format!(
                    "duplicate plugin id {}",
                    module.id
                )));
            }
            if module.path.as_os_str().is_empty() {
                return Err(LiveError::Config(format!(
                    "plugin {} must declare a non-empty path",
                    module.id
                )));
            }
            let sha256 = &module.sha256;
            if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(LiveError::Config(format!(
                    "plugin {} sha256 must be 64 hex characters",
                    module.id
                )));
            }
        }
        Ok(())
    }

    pub fn auth_token(&self) -> Option<String> {
        env::var(&self.auth.token_env).ok()
    }

    fn apply_environment(&mut self) -> Result<()> {
        if let Ok(value) = env::var("HOLOGRAM_LISTEN") {
            self.server.listen = value;
        }
        if let Ok(value) = env::var("HOLOGRAM_MAX_RPC_BYTES") {
            self.server.max_rpc_bytes = value.parse().map_err(|error| {
                LiveError::Config(format!(
                    "HOLOGRAM_MAX_RPC_BYTES must be a positive byte count: {error}"
                ))
            })?;
        }
        if let Ok(value) = env::var("HOLOGRAM_REMOTE_ENDPOINT") {
            self.client.remote_endpoint = Some(value);
        }
        if let Ok(value) = env::var("HOLOGRAM_LOG") {
            self.tracing.filter = value;
        } else if let Ok(value) = env::var("RUST_LOG") {
            self.tracing.filter = value;
        }
        if let Ok(value) = env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            self.telemetry.endpoint = Some(value);
        }
        if let Ok(value) = env::var("OTEL_SERVICE_NAME") {
            self.telemetry.service_name = value;
        }
        if let Some(value) = env::var_os("HOLOGRAM_DATA_DIR") {
            self.paths.data_dir = PathBuf::from(value);
        }
        if let Some(value) = env::var_os("HOLOGRAM_STATE_DIR") {
            self.paths.state_dir = PathBuf::from(value);
        }
        if let Some(value) = env::var_os("HOLOGRAM_CACHE_DIR") {
            self.paths.cache_dir = PathBuf::from(value);
        }
        Ok(())
    }

    fn expand_paths(&mut self) {
        self.paths.config_dir = expand_home(&self.paths.config_dir);
        self.paths.data_dir = expand_home(&self.paths.data_dir);
        self.paths.state_dir = expand_home(&self.paths.state_dir);
        self.paths.cache_dir = expand_home(&self.paths.cache_dir);
        for module in &mut self.plugins.modules {
            module.path = expand_home(&module.path);
        }
        if let Some(path) = &mut self.holo.development_grant {
            *path = expand_home(&*path);
            if path.is_relative() {
                *path = self.paths.config_dir.join(&*path);
            }
        }
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn validate_endpoint(endpoint: &str, local: bool) -> Result<()> {
    if endpoint.starts_with("https://") {
        return Ok(());
    }
    if endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://[::1]")
    {
        return Ok(());
    }
    let kind = if local { "local" } else { "remote" };
    Err(LiveError::Config(format!(
        "{kind} endpoint must use HTTPS unless it is loopback: {endpoint}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_uniform_config_directory() {
        assert!(AppConfig::default_path().ends_with(".config/hologram/live.toml"));
    }

    #[test]
    fn insecure_remote_endpoint_is_rejected() {
        let mut config = AppConfig::default();
        config.client.remote_endpoint = Some("http://example.com".to_owned());
        assert!(config.validate().is_err());
    }

    #[test]
    fn default_config_enables_the_builtin_module_catalogue() {
        assert_eq!(
            ModulesConfig::default().enabled,
            crate::modules::default_builtin_ids()
        );
    }

    #[test]
    fn noncurrent_configuration_schema_is_rejected() {
        let config = AppConfig {
            schema_version: 1,
            ..AppConfig::default()
        };
        let error = config.validate().expect_err("old schema");
        assert!(error
            .to_string()
            .contains("unsupported configuration schema 1"));
    }

    /// Each test gets its own directory so nothing races on a shared path.
    fn scratch_config(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = env::temp_dir().join(format!("hologram-config-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let path = dir.join("live.toml");
        std::fs::write(&path, body).expect("write scratch config");
        (dir, path)
    }

    const LEGACY: &[RetiredKey] = &[
        RetiredKey {
            path: "server.legacy_timeout_secs",
            removed_in: 2,
        },
        RetiredKey {
            path: "legacy",
            removed_in: 2,
        },
    ];

    /// The point of the mechanism: a file written before a field was removed
    /// still starts, instead of being refused by `deny_unknown_fields`.
    #[test]
    fn keys_retired_by_a_newer_schema_are_dropped_when_upgrading() {
        let source = r#"
schema_version = 1

[server]
listen = "127.0.0.1:11435"
legacy_timeout_secs = 5

[legacy]
whatever = true
"#;
        let config = AppConfig::from_source(source, true, Path::new("live.toml"), LEGACY)
            .expect("a file predating the removal must still load");
        // Everything the current schema still knows about survives the prune.
        assert_eq!(config.server.listen, "127.0.0.1:11435");
        assert_eq!(
            config.schema_version, 1,
            "migrate stamps the version, not this"
        );
    }

    /// Typo protection is the reason `deny_unknown_fields` was kept, so a file
    /// already at the current version must not have stray keys swept away.
    #[test]
    fn retired_keys_are_not_dropped_from_a_current_file() {
        let source = format!(
            "schema_version = {CURRENT_SCHEMA_VERSION}\n\n[server]\nlegacy_timeout_secs = 5\n"
        );
        let error = AppConfig::from_source(&source, true, Path::new("live.toml"), LEGACY)
            .expect_err("a current file keeps strict field checking");
        assert!(error.to_string().contains("legacy_timeout_secs"), "{error}");
    }

    #[test]
    fn bootstrap_parsing_never_prunes() {
        let source = "schema_version = 1\n\n[server]\nlegacy_timeout_secs = 5\n";
        let error = AppConfig::from_source(source, false, Path::new("live.toml"), LEGACY)
            .expect_err("the bootstrap read stays strict");
        assert!(error.to_string().contains("legacy_timeout_secs"), "{error}");
    }

    #[test]
    fn pruning_reports_only_the_paths_it_actually_removed() {
        let mut table =
            toml::from_str::<toml::Table>("schema_version = 1\n\n[server]\nlisten = \"x\"\n")
                .expect("parse table");

        let dropped = prune_retired_keys(&mut table, 1, LEGACY);

        assert!(
            dropped.is_empty(),
            "nothing retired was present: {dropped:?}"
        );
        assert!(table.contains_key("server"), "untouched keys stay");
    }

    #[test]
    fn pruning_skips_keys_retired_at_or_before_the_files_version() {
        let mut table = toml::from_str::<toml::Table>("[server]\nlegacy_timeout_secs = 5\n")
            .expect("parse table");

        // The file is already at 2, so a key retired in 2 is not swept.
        let dropped = prune_retired_keys(&mut table, 2, LEGACY);

        assert!(dropped.is_empty(), "{dropped:?}");
    }

    #[test]
    fn dotted_removal_handles_nested_and_top_level_paths() {
        let mut table = toml::from_str::<toml::Table>("[a]\nb = 1\nkeep = 2\n\n[top]\nx = 1\n")
            .expect("parse table");

        assert!(remove_dotted(&mut table, "a.b"));
        assert!(remove_dotted(&mut table, "top"));
        assert!(
            !remove_dotted(&mut table, "a.missing"),
            "absent key reports false"
        );
        assert!(
            !remove_dotted(&mut table, "missing.deep"),
            "absent parent reports false"
        );

        assert!(!table.contains_key("top"));
        let a = table
            .get("a")
            .and_then(toml::Value::as_table)
            .expect("table a");
        assert!(!a.contains_key("b"));
        assert!(a.contains_key("keep"), "siblings survive");
    }

    #[test]
    fn older_configuration_is_upgraded_and_rewritten() {
        let (dir, path) = scratch_config("upgrade", "schema_version = 1\nrole = \"node\"\n");

        let (config, loaded) = AppConfig::load(Some(&path)).expect("older config must load");
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded, path);

        let rewritten = std::fs::read_to_string(&path).expect("read rewritten config");
        assert!(
            rewritten.contains(&format!("schema_version = {CURRENT_SCHEMA_VERSION}")),
            "{rewritten}"
        );
        // The upgrade is written out in full, so the next start reads a file
        // that no longer depends on defaulting to fill the gaps.
        assert!(rewritten.contains("[inference]"), "{rewritten}");
        assert!(rewritten.contains("[plugins]"), "{rewritten}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The upgrade must round-trip what the user wrote, not what the process
    /// resolved it to. `~` expansion happens after the rewrite, so an expanded
    /// absolute path must never reach the file.
    #[test]
    fn upgrade_persists_only_what_the_user_wrote() {
        let (dir, path) = scratch_config(
            "no-leak",
            "schema_version = 1\n\n[paths]\ndata_dir = \"~/hologram-data\"\n",
        );

        let (config, _) = AppConfig::load(Some(&path)).expect("older config must load");
        assert!(
            !config.paths.data_dir.starts_with("~"),
            "in-memory path stays unexpanded: {}",
            config.paths.data_dir.display()
        );

        let rewritten = std::fs::read_to_string(&path).expect("read rewritten config");
        assert!(
            rewritten.contains("~/hologram-data"),
            "expansion leaked into the file: {rewritten}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn newer_configuration_schema_is_refused() {
        let (dir, path) = scratch_config("newer", "schema_version = 99\n");

        let error = AppConfig::load(Some(&path)).expect_err("newer schema must fail");
        assert!(
            error.to_string().contains("newer than this build supports"),
            "{error}"
        );
        let untouched = std::fs::read_to_string(&path).expect("read config");
        assert!(untouched.contains("99"), "{untouched}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn configuration_schema_below_the_supported_floor_is_refused() {
        let (dir, path) = scratch_config("floor", "schema_version = 0\n");

        let error = AppConfig::load(Some(&path)).expect_err("schema 0 must fail");
        assert!(error.to_string().contains("predates"), "{error}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn current_configuration_is_left_untouched() {
        let body =
            format!("schema_version = {CURRENT_SCHEMA_VERSION}\n# a comment worth keeping\n");
        let (dir, path) = scratch_config("current", &body);

        AppConfig::load(Some(&path)).expect("current config loads");

        let after = std::fs::read_to_string(&path).expect("read config");
        assert_eq!(after, body, "a current config must not be rewritten");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The bootstrap read happens before a tracing subscriber exists, so it
    /// must not perform an upgrade whose log line would be dropped.
    #[test]
    fn bootstrap_load_never_rewrites_the_file() {
        let body = "schema_version = 1\n";
        let (dir, path) = scratch_config("bootstrap", body);

        AppConfig::load_for_bootstrap(Some(&path)).expect_err("stale config fails validate");

        let after = std::fs::read_to_string(&path).expect("read config");
        assert_eq!(after, body, "bootstrap must leave the file alone");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_section_fields_fall_back_to_defaults() {
        let mut value = toml::Value::try_from(AppConfig::default()).expect("serialize config");
        value
            .get_mut("server")
            .and_then(toml::Value::as_table_mut)
            .expect("server table")
            .remove("max_rpc_bytes");
        let source = toml::to_string(&value).expect("encode config");
        let config = toml::from_str::<AppConfig>(&source).expect("field added after this file");
        assert_eq!(
            config.server.max_rpc_bytes,
            ServerConfig::default().max_rpc_bytes
        );
        config.validate().expect("defaulted config validates");
    }

    /// A config written before `[inference]`, `[holo]`, and `[plugins]` existed
    /// must still boot: `schema_version` is unchanged, so the file is current
    /// as far as its author knew.
    #[test]
    fn configuration_predating_a_whole_section_still_loads() {
        let source = r#"
schema_version = 2
role = "node"

[server]
listen = "127.0.0.1:11435"

[modules]
enabled = ["dev.hologram.live.system"]
"#;
        let config = toml::from_str::<AppConfig>(source).expect("older config must load");
        assert_eq!(config.inference.engine, InferenceConfig::default().engine);
        assert!(config.holo.resident.is_empty());
        assert!(!config.plugins.enabled);
        assert_eq!(config.server.listen, "127.0.0.1:11435");
        assert_eq!(config.modules.enabled, ["dev.hologram.live.system"]);
        config.validate().expect("older config validates");
    }

    #[test]
    fn missing_schema_version_is_rejected() {
        let error = toml::from_str::<AppConfig>("role = \"node\"\n")
            .expect_err("schema_version anchors compatibility and is never guessed");
        assert!(error.to_string().contains("schema_version"), "{error}");
    }

    #[test]
    fn unknown_configuration_fields_are_still_rejected() {
        let mut value = toml::Value::try_from(AppConfig::default()).expect("serialize config");
        value
            .get_mut("server")
            .and_then(toml::Value::as_table_mut)
            .expect("server table")
            .insert("listne".to_owned(), toml::Value::String("oops".to_owned()));
        let source = toml::to_string(&value).expect("encode config");
        let error = toml::from_str::<AppConfig>(&source).expect_err("typo must fail");
        assert!(error.to_string().contains("listne"), "{error}");
    }

    #[test]
    fn plugin_entries_still_require_every_field() {
        let source = r#"
schema_version = 2

[plugins]
enabled = true

[[plugins.modules]]
id = "dev.example.plugin"
path = "/usr/local/bin/plugin"
"#;
        let error = toml::from_str::<AppConfig>(source)
            .expect_err("hand-authored list entries stay strict");
        assert!(error.to_string().contains("sha256"), "{error}");
    }

    #[test]
    fn inference_defaults_to_the_local_echo_engine() {
        let config = AppConfig::default();
        assert_eq!(config.inference.engine, "echo");
        assert!(config.inference.default_model.is_empty());
        assert_eq!(config.inference.weightc_path, "weightc");
        assert_eq!(config.inference.ollama_endpoint, "http://127.0.0.1:11434");
        assert_eq!(config.inference.request_timeout_secs, 300);
        assert!(!config.inference.resident_sessions);
        assert_eq!(config.inference.max_resident_sessions, 4);
        config.validate().expect("default config validates");
    }

    #[test]
    fn zero_max_resident_sessions_is_rejected() {
        let mut config = AppConfig::default();
        config.inference.max_resident_sessions = 0;
        let error = config.validate().expect_err("must fail");
        assert!(
            error.to_string().contains("max_resident_sessions"),
            "{error}"
        );
    }

    #[test]
    fn unknown_inference_engine_is_rejected() {
        let mut config = AppConfig::default();
        config.inference.engine = "surprise".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn current_config_can_intentionally_disable_chat() {
        let mut config = AppConfig::default();
        config
            .modules
            .enabled
            .retain(|id| id != "dev.hologram.live.chat");

        config.validate().expect("current config");
        assert!(!config
            .modules
            .enabled
            .iter()
            .any(|id| id == "dev.hologram.live.chat"));
    }

    #[test]
    fn plugins_default_to_disabled_with_an_empty_allowlist() {
        let config = AppConfig::default();
        assert!(!config.plugins.enabled);
        assert!(config.plugins.modules.is_empty());
        config.validate().expect("default config validates");
    }

    #[test]
    fn holo_development_grants_are_explicit_and_resolve_from_config_dir() {
        let mut config = AppConfig::default();
        assert!(config.holo.development_grant.is_none());
        config.paths.config_dir = PathBuf::from("/tmp/hologram-config");
        config.holo.development_grant = Some(PathBuf::from("development-grant.json"));

        config.expand_paths();

        assert_eq!(
            config.holo.development_grant,
            Some(PathBuf::from("/tmp/hologram-config/development-grant.json"))
        );
    }

    #[test]
    fn holo_development_grant_is_rejected_on_non_loopback_servers() {
        let mut config = AppConfig::default();
        config.server.listen = "0.0.0.0:11435".to_owned();
        config.auth.required = true;
        config.holo.development_grant = Some(PathBuf::from("grant.json"));

        let error = config
            .validate()
            .expect_err("development grant must stay local");
        assert!(error.to_string().contains("loopback"), "{error}");
    }

    #[test]
    fn holo_resident_declarations_parse_and_validate() {
        let source = format!(
            "{}\n[[holo.resident]]\nkappa = \"blake3:{}\"\n",
            toml::to_string(&AppConfig::default()).expect("encode default config"),
            "ab".repeat(32)
        );
        let config = toml::from_str::<AppConfig>(&source).expect("parse resident declaration");
        assert_eq!(config.holo.resident.len(), 1);
        assert_eq!(
            config.holo.resident[0].kappa,
            format!("blake3:{}", "ab".repeat(32))
        );
        config.validate().expect("resident declaration validates");
    }

    #[test]
    fn holo_resident_declarations_default_to_empty() {
        let config = AppConfig::default();
        assert!(config.holo.resident.is_empty());
        config.validate().expect("default config validates");
    }

    #[test]
    fn holo_resident_kappas_must_be_well_formed() {
        let mut config = AppConfig::default();
        config.holo.resident.push(HoloResidentConfig {
            kappa: "sha256:abc".to_owned(),
        });
        let error = config.validate().expect_err("prefix must fail");
        assert!(error.to_string().contains("blake3:"), "{error}");

        let mut config = AppConfig::default();
        config.holo.resident.push(HoloResidentConfig {
            kappa: "blake3:abc".to_owned(),
        });
        let error = config.validate().expect_err("length must fail");
        assert!(error.to_string().contains("64 hex"), "{error}");
    }

    #[test]
    fn holo_resident_kappas_must_be_unique() {
        let mut config = AppConfig::default();
        let kappa = format!("blake3:{}", "cd".repeat(32));
        config.holo.resident.push(HoloResidentConfig {
            kappa: kappa.clone(),
        });
        config.holo.resident.push(HoloResidentConfig { kappa });
        let error = config.validate().expect_err("duplicate must fail");
        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    #[test]
    fn plugin_entries_require_a_well_formed_sha256() {
        let mut config = plugin_config();
        config.plugins.modules[0].sha256 = "not-hex".to_owned();
        let error = config.validate().expect_err("must fail");
        assert!(error.to_string().contains("64 hex"), "{error}");

        let mut config = plugin_config();
        config.plugins.modules[0].sha256 = "ab".repeat(32).to_uppercase();
        config.validate().expect("uppercase hex is accepted");
    }

    #[test]
    fn plugin_ids_must_be_unique_and_not_shadow_builtins() {
        let mut config = plugin_config();
        let duplicate = config.plugins.modules[0].clone();
        config.plugins.modules.push(duplicate);
        let error = config.validate().expect_err("duplicate id must fail");
        assert!(error.to_string().contains("duplicate plugin id"), "{error}");

        let mut config = plugin_config();
        config.plugins.modules[0].id = "dev.hologram.live.system".to_owned();
        let error = config.validate().expect_err("builtin id must fail");
        assert!(error.to_string().contains("builtin module id"), "{error}");
    }

    #[test]
    fn plugin_entries_require_an_id_and_a_path() {
        let mut config = plugin_config();
        config.plugins.modules[0].id = " ".to_owned();
        assert!(config.validate().is_err());

        let mut config = plugin_config();
        config.plugins.modules[0].path = PathBuf::new();
        assert!(config.validate().is_err());
    }

    fn plugin_config() -> AppConfig {
        let mut config = AppConfig::default();
        config.plugins.enabled = true;
        config.plugins.modules.push(PluginModuleConfig {
            id: "com.example.echo".to_owned(),
            path: PathBuf::from("/opt/plugins/echo"),
            sha256: "ab".repeat(32),
        });
        config
    }
}
