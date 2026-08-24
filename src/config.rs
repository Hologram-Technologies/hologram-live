use crate::error::{LiveError, Result};
use crate::util::{atomic_write, expand_home, home_dir};
use serde::{Deserialize, Serialize};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const CURRENT_SCHEMA_VERSION: u32 = 2;
const CHAT_MODULE_ID: &str = "dev.hologram.live.chat";
const V1_DEFAULT_MODULES: &[&str] = &[
    "dev.hologram.live.system",
    "dev.hologram.live.kappa-registry",
    "dev.hologram.live.files",
    "dev.hologram.live.holo",
    "dev.hologram.live.history",
    "dev.hologram.live.control-plane",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
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
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub role: ServerRole,
    pub paths: PathsConfig,
    pub server: ServerConfig,
    pub client: ClientConfig,
    pub auth: AuthConfig,
    pub tracing: TracingConfig,
    pub telemetry: TelemetryConfig,
    pub modules: ModulesConfig,
    pub update: UpdateConfig,
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

    pub fn load(path: Option<&Path>) -> Result<(Self, PathBuf)> {
        let path = path.map_or_else(Self::default_path, expand_home);
        let exists = path.exists();
        let mut config = if exists {
            let source =
                std::fs::read_to_string(&path).map_err(|error| LiveError::io(&path, error))?;
            toml::from_str::<Self>(&source)?
        } else {
            Self::default()
        };
        if config.migrate()? && exists {
            let bytes = toml::to_string_pretty(&config)?;
            atomic_write(&path, bytes.as_bytes())?;
        }
        config.apply_environment();
        config.expand_paths();
        config.validate()?;
        Ok((config, path))
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
        Ok(())
    }

    pub fn auth_token(&self) -> Option<String> {
        env::var(&self.auth.token_env).ok()
    }

    fn migrate(&mut self) -> Result<bool> {
        match self.schema_version {
            CURRENT_SCHEMA_VERSION => Ok(false),
            1 => {
                if is_v1_default_module_set(&self.modules.enabled) {
                    self.modules.enabled.push(CHAT_MODULE_ID.to_owned());
                }
                self.schema_version = CURRENT_SCHEMA_VERSION;
                Ok(true)
            }
            other => Err(LiveError::Config(format!(
                "unsupported configuration schema {other}"
            ))),
        }
    }

    fn apply_environment(&mut self) {
        if let Ok(value) = env::var("HOLOGRAM_LISTEN") {
            self.server.listen = value;
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
    }

    fn expand_paths(&mut self) {
        self.paths.config_dir = expand_home(&self.paths.config_dir);
        self.paths.data_dir = expand_home(&self.paths.data_dir);
        self.paths.state_dir = expand_home(&self.paths.state_dir);
        self.paths.cache_dir = expand_home(&self.paths.cache_dir);
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn is_v1_default_module_set(enabled: &[String]) -> bool {
    enabled.len() == V1_DEFAULT_MODULES.len()
        && V1_DEFAULT_MODULES
            .iter()
            .all(|expected| enabled.iter().any(|actual| actual == expected))
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
    fn v1_default_config_migrates_to_enable_chat() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("live.toml");
        let mut legacy = AppConfig {
            schema_version: 1,
            ..AppConfig::default()
        };
        legacy.modules.enabled = V1_DEFAULT_MODULES
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        std::fs::write(
            &path,
            toml::to_string_pretty(&legacy).expect("encode legacy"),
        )
        .expect("write legacy");

        let (loaded, _) = AppConfig::load(Some(&path)).expect("load migrated config");

        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(loaded.modules.enabled.iter().any(|id| id == CHAT_MODULE_ID));
        let persisted = std::fs::read_to_string(path).expect("read migrated config");
        assert!(persisted.contains("schema_version = 2"));
        assert!(persisted.contains(CHAT_MODULE_ID));
    }

    #[test]
    fn v1_custom_module_selection_is_preserved_during_migration() {
        let mut legacy = AppConfig {
            schema_version: 1,
            ..AppConfig::default()
        };
        legacy.modules.enabled = vec!["dev.hologram.live.system".to_owned()];

        assert!(legacy.migrate().expect("migrate"));
        assert_eq!(legacy.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            legacy.modules.enabled,
            vec!["dev.hologram.live.system".to_owned()]
        );
    }

    #[test]
    fn v2_config_can_intentionally_disable_chat() {
        let mut config = AppConfig::default();
        config.modules.enabled.retain(|id| id != CHAT_MODULE_ID);

        assert!(!config.migrate().expect("already current"));
        assert!(!config.modules.enabled.iter().any(|id| id == CHAT_MODULE_ID));
    }
}
