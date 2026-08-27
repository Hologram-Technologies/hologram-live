use crate::error::{LiveError, Result};
use crate::util::{atomic_write, expand_home, home_dir};
use serde::{Deserialize, Serialize};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const CURRENT_SCHEMA_VERSION: u32 = 2;

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
#[serde(deny_unknown_fields)]
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
    pub inference: InferenceConfig,
    pub holo: HoloConfig,
    pub plugins: PluginsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: String,
    pub max_rpc_bytes: usize,
    pub max_http_body_bytes: usize,
    pub graceful_shutdown_secs: u64,
    pub actor_mailbox_capacity: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub preference: TargetPreference,
    pub local_endpoint: String,
    pub remote_endpoint: Option<String>,
    pub allow_read_fallback: bool,
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    pub required: bool,
    pub token_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    pub filter: String,
    pub format: String,
    pub include_target: bool,
    pub include_thread_ids: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    /// Enables creation of application metrics and trace context.
    pub enabled: bool,
    /// OTLP/gRPC collector endpoint. No data leaves the process when unset.
    pub endpoint: Option<String>,
    pub service_name: String,
    pub export_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulesConfig {
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    pub manifest_url: Option<String>,
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
        config.apply_environment()?;
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

    #[test]
    fn missing_current_configuration_fields_are_rejected() {
        let mut value = toml::Value::try_from(AppConfig::default()).expect("serialize config");
        value
            .get_mut("server")
            .and_then(toml::Value::as_table_mut)
            .expect("server table")
            .remove("max_rpc_bytes");
        let source = toml::to_string(&value).expect("encode config");
        let error =
            toml::from_str::<AppConfig>(&source).expect_err("missing current field must fail");
        assert!(error.to_string().contains("max_rpc_bytes"), "{error}");
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
