//! The reference registry's configuration, read as the reference reads it:
//! `config.yml` first, then `REGISTRY_*` environment variables over it.
//!
//! Every key is classed in [`table`]. A key the table refuses, or does not
//! know, stops the start with the key and the reason: a setting is never
//! ignored silently, because an operator who set `http.tls` and got plain
//! HTTP would not know.

pub mod table;

use crate::config::{AppConfig, TracingConfig};
use crate::error::{LiveError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use table::{classify, Class};
use yaml_rust2::{Yaml, YamlLoader};

/// Sections whose children are free-form names, not documented keys.
const OPEN_SECTIONS: [&str; 2] = ["http.headers", "log.fields"];

/// The reference's own way to name its file when none is given.
const CONFIGURATION_PATH: &str = "REGISTRY_CONFIGURATION_PATH";

/// The registry's settings as flat key paths, `http.addr` = `:5000`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrySettings {
    pub values: BTreeMap<String, String>,
    /// Accepted but not in effect yet: logged once at start.
    pub pending: Vec<String>,
    /// `REGISTRY_*` variables that are not settings but are expected in a
    /// pod (Kubernetes service links): skipped, logged once at start.
    pub skipped: Vec<String>,
}

impl RegistrySettings {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// A boolean as the reference's YAML 1.1 parser reads one: `true`, `yes`
    /// and `on` are true.
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.get(key).map(|value| {
            let value = value.trim();
            ["true", "yes", "on"]
                .iter()
                .any(|word| value.eq_ignore_ascii_case(word))
        })
    }

    /// Every value under a section, keyed by the rest of the path.
    pub fn section(&self, prefix: &str) -> Vec<(&str, &str)> {
        let start = format!("{prefix}.");
        self.values
            .iter()
            .filter_map(|(key, value)| Some((key.strip_prefix(&start)?, value.as_str())))
            .collect()
    }
}

static INSTALLED: OnceLock<RegistrySettings> = OnceLock::new();

/// The settings the running registry was started with, when it was started
/// from a registry configuration.
pub fn installed() -> Option<&'static RegistrySettings> {
    INSTALLED.get()
}

/// Read the file (or the one `REGISTRY_CONFIGURATION_PATH` names), then the
/// environment over it, and class every key.
///
/// # Errors
///
/// `LiveError::Config` naming the key and the reason, for a key that is
/// refused or unknown, a value that is refused, or a file that is not YAML.
pub fn load(
    file: Option<&Path>,
    environment: impl IntoIterator<Item = (String, String)>,
) -> Result<RegistrySettings> {
    let environment: Vec<(String, String)> = environment.into_iter().collect();
    let named = environment
        .iter()
        .find(|(name, _)| name == CONFIGURATION_PATH)
        .map(|(_, value)| PathBuf::from(value));
    let file = file.map(Path::to_path_buf).or(named);
    let mut values = BTreeMap::new();
    if let Some(path) = file.as_deref() {
        let text = std::fs::read_to_string(path).map_err(|error| LiveError::io(path, error))?;
        flatten_yaml(&text, &mut values).map_err(|error| {
            LiveError::Config(format!(
                "registry configuration {}: {error}",
                path.display()
            ))
        })?;
    }
    let mut skipped = Vec::new();
    for (name, value) in environment {
        if name == CONFIGURATION_PATH {
            continue;
        }
        if is_service_link(&name) {
            skipped.push(name);
            continue;
        }
        if let Some(key) = env_key(&name, &value)? {
            values.insert(key, value);
        }
    }
    let mut pending = Vec::new();
    for (key, value) in &values {
        check(key, value, &mut pending)?;
    }
    // The reference's TLS listener needs both files.
    match (
        values.contains_key("http.tls.certificate"),
        values.contains_key("http.tls.key"),
    ) {
        (true, false) => {
            return Err(LiveError::Config(
                "registry setting http.tls.certificate is set without http.tls.key".to_owned(),
            ))
        }
        (false, true) => {
            return Err(LiveError::Config(
                "registry setting http.tls.key is set without http.tls.certificate".to_owned(),
            ))
        }
        _ => {}
    }
    Ok(RegistrySettings {
        values,
        pending,
        skipped,
    })
}

/// Kubernetes puts `<SERVICE>_PORT`, `<SERVICE>_PORT_<n>_TCP…` and
/// `<SERVICE>_SERVICE_HOST`, `…_PORT…` into every pod beside a Service: one
/// named `registry` yields `REGISTRY_PORT` and the rest. They are not settings.
fn is_service_link(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("REGISTRY_") else {
        return false;
    };
    rest == "PORT"
        || rest.starts_with("PORT_")
        || rest.starts_with("SERVICE_HOST")
        || rest.starts_with("SERVICE_PORT")
}

fn check(key: &str, value: &str, pending: &mut Vec<String>) -> Result<()> {
    let refuse = |reason: &str| {
        Err(LiveError::Config(format!(
            "registry setting {key} {reason}"
        )))
    };
    let Some(entry) = classify(key) else {
        return refuse("is not a setting of the reference registry this one replaces");
    };
    match entry.class {
        Class::Supported | Class::Ignored(_) => {}
        Class::Pending(_) => pending.push(key.to_owned()),
        Class::Refused(reason) => return refuse(reason),
    }
    // Values the table cannot express by key alone.
    match (key, value.trim()) {
        ("log.formatter", "logstash") => refuse("logstash is not supported: use json"),
        ("log.formatter", other) if !matches!(other, "text" | "json") => {
            refuse("must be text or json")
        }
        ("http.tls.minimumtls", version) => match crate::tls::MinimumTls::parse(version) {
            Ok(_) => Ok(()),
            Err(reason) => refuse(&reason),
        },
        ("storage.cache.blobdescriptor", "redis") => {
            refuse("redis is not supported: v1 is one writer")
        }
        _ => match key.strip_prefix("http.headers.") {
            // A header that cannot be sent is refused, not dropped.
            Some(name) if axum::http::HeaderName::from_bytes(name.as_bytes()).is_err() => {
                refuse("is not a valid header name")
            }
            Some(_) if axum::http::HeaderValue::from_str(header_value(value)).is_err() => {
                refuse("has a value that is not a valid header value")
            }
            _ => Ok(()),
        },
    }
}

/// A header value as the reference writes it in YAML: a list, `[nosniff]`,
/// sent as one comma-joined value.
pub fn header_value(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim()
}

/// The key a `REGISTRY_*` variable sets, or `None` for any other variable.
fn env_key(name: &str, value: &str) -> Result<Option<String>> {
    let Some(rest) = name.strip_prefix("REGISTRY_") else {
        return Ok(None);
    };
    // Type selectors, as the deployment guide sets them.
    match rest {
        "AUTH" => {
            return match value.trim() {
                // Recorded as the section itself: the selector alone asks for a
                // login, and the start refuses it without a path and a realm,
                // as the reference does.
                "htpasswd" => Ok(Some("auth.htpasswd".to_owned())),
                other => Err(LiveError::Config(format!(
                    "REGISTRY_AUTH={other}: only htpasswd is supported"
                ))),
            };
        }
        "STORAGE" => {
            return match value.trim() {
                "filesystem" => Ok(None),
                other => Err(LiveError::Config(format!(
                    "REGISTRY_STORAGE={other}: only filesystem is supported"
                ))),
            };
        }
        _ => {}
    }
    // Free-form children keep the name as written after the section.
    for section in OPEN_SECTIONS {
        let prefix = format!("{}_", section.to_ascii_uppercase().replace('.', "_"));
        if let Some(child) = rest.strip_prefix(&prefix) {
            let child = if section == "log.fields" {
                child.to_ascii_lowercase()
            } else {
                child.to_owned()
            };
            return Ok(Some(format!("{section}.{child}")));
        }
    }
    // Every documented key, matched whole: an underscore inside a name (for
    // example max_retries) cannot be told from a level break any other way.
    let wanted = rest.to_ascii_lowercase();
    if let Some(key) = documented_keys().find(|key| key.replace('.', "_") == wanted) {
        return Ok(Some(key.to_owned()));
    }
    Err(LiveError::Config(format!(
        "{name} is not a setting of the reference registry this one replaces"
    )))
}

fn documented_keys() -> impl Iterator<Item = &'static str> {
    include_str!("../../tests/fixtures/registry-config-keys.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Flatten a YAML mapping into `a.b.c` = scalar. A list is kept as its items
/// joined by commas; the only v1 setting that takes one is a header.
fn flatten_yaml(text: &str, out: &mut BTreeMap<String, String>) -> std::result::Result<(), String> {
    let documents = YamlLoader::load_from_str(text).map_err(|error| error.to_string())?;
    let Some(root) = documents.into_iter().next() else {
        return Ok(());
    };
    walk("", &root, out)
}

/// One YAML node into `prefix.key` = scalar entries.
fn walk(
    prefix: &str,
    node: &Yaml,
    out: &mut BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match node {
        Yaml::Hash(map) => {
            for (key, value) in map {
                let name = match key {
                    Yaml::String(text) => text.clone(),
                    Yaml::Integer(number) => number.to_string(),
                    other => return Err(format!("a key that is not text: {other:?}")),
                };
                let path = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}.{name}")
                };
                walk(&path, value, out)?;
            }
            Ok(())
        }
        // A key with no value sets nothing, as in the reference.
        Yaml::Null => Ok(()),
        Yaml::String(text) | Yaml::Real(text) => {
            out.insert(prefix.to_owned(), text.clone());
            Ok(())
        }
        Yaml::Integer(number) => {
            out.insert(prefix.to_owned(), number.to_string());
            Ok(())
        }
        Yaml::Boolean(flag) => {
            out.insert(prefix.to_owned(), flag.to_string());
            Ok(())
        }
        Yaml::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    Yaml::String(text) => text.clone(),
                    other => format!("{other:?}"),
                })
                .collect();
            out.insert(prefix.to_owned(), parts.join(","));
            Ok(())
        }
        Yaml::Alias(_) | Yaml::BadValue => Err(format!("{prefix}: a value that cannot be read")),
    }
}

/// Logging from the registry's settings: `log.level`, and `log.formatter`,
/// where the reference's `text` is this server's `compact` line format.
/// Used before the process's logging starts, so registry mode never takes its
/// logging from a user's configuration file.
pub fn tracing_config(settings: &RegistrySettings, mut tracing: TracingConfig) -> TracingConfig {
    let level = settings
        .get("log.level")
        .or_else(|| settings.get("loglevel"));
    if let Some(level) = level {
        level.trim().clone_into(&mut tracing.filter);
    }
    match settings.get("log.formatter").map(str::trim) {
        Some("json") => "json".clone_into(&mut tracing.format),
        Some(_) => "compact".clone_into(&mut tracing.format),
        None => {}
    }
    tracing
}

/// Apply what the server itself owns, and keep the rest for the registry
/// module. Called once, before the configuration is validated.
pub fn apply(settings: RegistrySettings, config: &mut AppConfig) {
    // `:5000` means every interface, as in Go. With no `http.addr`, the
    // image's port (plan D1).
    let addr = settings.get("http.addr").unwrap_or(":5000");
    config.server.listen = match addr.strip_prefix(':') {
        Some(port) => format!("0.0.0.0:{port}"),
        None => addr.to_owned(),
    };
    // Everything the server keeps lives on the registry's volume, under
    // `live/`, so a container needs no home directory and a developer's home
    // is never written (FR-S07).
    let root = PathBuf::from(
        settings
            .get("storage.filesystem.rootdirectory")
            .unwrap_or("/var/lib/registry"),
    );
    root.clone_into(&mut config.paths.data_dir);
    config.paths.config_dir = root.join("live/config");
    config.paths.state_dir = root.join("live/state");
    config.paths.cache_dir = root.join("live/cache");
    config.tracing = tracing_config(&settings, std::mem::take(&mut config.tracing));
    if let (Some(certificate), Some(key)) = (
        settings.get("http.tls.certificate"),
        settings.get("http.tls.key"),
    ) {
        config.tls = Some(crate::tls::TlsSettings {
            certificate: PathBuf::from(certificate.trim()),
            key: PathBuf::from(key.trim()),
            // Checked when the settings were loaded.
            minimum: crate::tls::MinimumTls::parse(
                settings.get("http.tls.minimumtls").unwrap_or(""),
            )
            .unwrap_or(crate::tls::MinimumTls::Tls12),
        });
    }
    for key in &settings.pending {
        tracing::warn!(
            setting = key.as_str(),
            "registry setting accepted, not in effect yet"
        );
    }
    for name in &settings.skipped {
        tracing::info!(
            variable = name.as_str(),
            "not a registry setting (a Kubernetes service link); skipped"
        );
    }
    let _ = INSTALLED.set(settings);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn load_text(text: &str, environment: &[(&str, &str)]) -> Result<RegistrySettings> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, text).expect("write");
        load(Some(&path), env(environment))
    }

    /// Every key the reference documents is classed, or is a section whose
    /// children are; every refused key stops the start naming itself; every
    /// accepted one loads.
    #[test]
    fn walks_every_documented_key() {
        let keys: Vec<&str> = documented_keys().collect();
        assert!(keys.len() > 200, "the fixture holds the documented list");
        for key in &keys {
            let is_section = keys
                .iter()
                .any(|other| other.starts_with(&format!("{key}.")));
            let Some(entry) = classify(key) else {
                assert!(is_section, "{key} is documented and not classed");
                continue;
            };
            if is_section {
                continue;
            }
            let mut values = BTreeMap::new();
            values.insert((*key).to_owned(), sample(key).to_owned());
            let mut pending = Vec::new();
            let checked = values
                .iter()
                .try_for_each(|(k, v)| check(k, v, &mut pending));
            match entry.class {
                Class::Refused(reason) => {
                    let message = checked.expect_err(key).to_string();
                    assert!(
                        message.contains(key) && message.contains(reason),
                        "{message}"
                    );
                }
                _ => assert!(checked.is_ok(), "{key}: {checked:?}"),
            }
        }
    }

    fn sample(key: &str) -> &'static str {
        match key {
            "log.formatter" => "json",
            "storage.cache.blobdescriptor" => "inmemory",
            _ => "x",
        }
    }

    /// The official image's default file, verbatim
    /// (distribution/distribution-library-image, config-example.yml).
    const IMAGE_DEFAULT: &str = "version: 0.1
log:
  level: debug
  fields:
    service: registry
    environment: development
storage:
    delete:
      enabled: true
    cache:
        blobdescriptor: inmemory
    filesystem:
        rootdirectory: /var/lib/registry
    tag:
      concurrencylimit: 5
http:
    addr: :5000
    debug:
        addr: :5001
        prometheus:
            enabled: true
            path: /metrics
health:
  storagedriver:
    enabled: true
    interval: 10s
    threshold: 3
";

    #[test]
    fn the_reference_images_own_default_file_loads() {
        let settings = load_text(IMAGE_DEFAULT, &[]).expect("the image's default file");
        assert_eq!(settings.get("http.addr"), Some(":5000"));
        assert_eq!(settings.flag("storage.delete.enabled"), Some(true));
        assert!(settings.pending.iter().any(|key| key == "http.debug.addr"));
        assert!(settings
            .pending
            .iter()
            .any(|key| key.starts_with("health.storagedriver")));
        let mut config = AppConfig::default();
        apply(settings, &mut config);
        assert_eq!(config.server.listen, "0.0.0.0:5000");
        assert_eq!(
            config.paths.data_dir,
            std::path::PathBuf::from("/var/lib/registry")
        );
        assert_eq!(
            config.paths.state_dir,
            std::path::PathBuf::from("/var/lib/registry/live/state"),
            "nothing under a home directory"
        );
        assert_eq!(config.tracing.filter, "debug");
    }

    /// The image's `config.yml` is the reference's key for key, but for the
    /// one change DIFFERENCES.md lists (gate F item 2, in small).
    #[test]
    fn the_images_file_differs_from_the_references_only_in_log_level() {
        let ours = load_text(include_str!("../../apps/registry/config.yml"), &[])
            .expect("the image's file loads");
        let reference = load_text(IMAGE_DEFAULT, &[]).expect("the reference's file");
        let mut differs: Vec<&str> = ours
            .values
            .keys()
            .chain(reference.values.keys())
            .filter(|key| ours.get(key) != reference.get(key))
            .map(String::as_str)
            .collect();
        differs.dedup();
        assert_eq!(differs, ["log.level"]);
        assert_eq!(ours.get("log.level"), Some("info"));
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let settings = load_text(
            IMAGE_DEFAULT,
            &[
                ("REGISTRY_STORAGE_DELETE_ENABLED", "false"),
                ("PATH", "/bin"),
            ],
        )
        .expect("load");
        assert_eq!(settings.flag("storage.delete.enabled"), Some(false));
    }

    #[test]
    fn a_setting_that_is_not_built_or_not_supported_stops_the_start_by_name() {
        for (name, value, key) in [
            ("REGISTRY_STORAGE_S3_BUCKET", "x", "storage.s3.bucket"),
            (
                "REGISTRY_STORAGE_MAINTENANCE_READONLY_ENABLED",
                "true",
                "storage.maintenance.readonly",
            ),
            (
                "REGISTRY_PROXY_REMOTEURL",
                "https://registry-1.docker.io",
                "proxy.remoteurl",
            ),
        ] {
            let error = load(None, env(&[(name, value)]))
                .expect_err(name)
                .to_string();
            assert!(error.contains(key), "{error}");
        }
        let error = load(None, env(&[("REGISTRY_NOT_A_THING", "1")]))
            .expect_err("unknown")
            .to_string();
        assert!(error.contains("REGISTRY_NOT_A_THING"), "{error}");
        let error = load_text(
            "storage:\n  filesystem:\n    rootdirectory: /r\n  mystery: 1\n",
            &[],
        )
        .expect_err("unknown key")
        .to_string();
        assert!(error.contains("storage.mystery"), "{error}");
    }

    /// `http.tls` as the deployment guide sets it: both files, or neither;
    /// `minimumtls` 1.2 or 1.3, and the older versions refused by name.
    #[test]
    fn tls_takes_both_files_and_a_version_this_listener_offers() {
        let settings = load(
            None,
            env(&[
                ("REGISTRY_HTTP_TLS_CERTIFICATE", "/certs/domain.crt"),
                ("REGISTRY_HTTP_TLS_KEY", "/certs/domain.key"),
            ]),
        )
        .expect("the guide's two variables");
        let mut config = AppConfig::default();
        apply(settings, &mut config);
        let tls = config.tls.expect("TLS on");
        assert_eq!(tls.certificate, PathBuf::from("/certs/domain.crt"));
        assert_eq!(
            tls.minimum,
            crate::tls::MinimumTls::Tls12,
            "the reference's default"
        );

        for (alone, missing) in [
            ("REGISTRY_HTTP_TLS_CERTIFICATE", "http.tls.key"),
            ("REGISTRY_HTTP_TLS_KEY", "http.tls.certificate"),
        ] {
            let error = load(None, env(&[(alone, "/x")]))
                .expect_err(alone)
                .to_string();
            assert!(error.contains(missing), "{error}");
        }
        let settings = load_text(
            "http:\n  tls:\n    certificate: /c\n    key: /k\n    minimumtls: tls1.3\n",
            &[],
        )
        .expect("tls1.3");
        let mut config = AppConfig::default();
        apply(settings, &mut config);
        assert_eq!(
            config.tls.expect("on").minimum,
            crate::tls::MinimumTls::Tls13
        );
        for old in ["tls1.0", "tls1.1", "ssl3"] {
            let text =
                format!("http:\n  tls:\n    certificate: /c\n    key: /k\n    minimumtls: {old}\n");
            let error = load_text(&text, &[]).expect_err(old).to_string();
            assert!(error.contains("http.tls.minimumtls"), "{error}");
        }
        let mut config = AppConfig::default();
        apply(load(None, env(&[])).expect("none"), &mut config);
        assert!(config.tls.is_none(), "no http.tls, no TLS");
    }

    #[test]
    fn what_many_compose_files_set_is_accepted() {
        // http.secret is in many real files, and the guide sets the selectors.
        let settings = load(
            None,
            env(&[
                ("REGISTRY_HTTP_SECRET", "s3cr3t"),
                ("REGISTRY_STORAGE", "filesystem"),
                ("REGISTRY_HTTP_ADDR", "0.0.0.0:5000"),
                ("REGISTRY_STORAGE_FILESYSTEM_ROOTDIRECTORY", "/data"),
                ("REGISTRY_HTTP_HEADERS_Access-Control-Allow-Origin", "*"),
            ]),
        )
        .expect("load");
        assert_eq!(
            settings.get("http.headers.Access-Control-Allow-Origin"),
            Some("*")
        );
        assert!(load(None, env(&[("REGISTRY_STORAGE", "s3")])).is_err());
        assert!(load(None, env(&[("REGISTRY_AUTH", "token")])).is_err());
    }

    #[test]
    fn review_of_99() {
        // text is the reference's default formatter
        let settings = load_text("log:\n  level: warn\n  formatter: text\n", &[]).expect("load");
        let tracing = tracing_config(&settings, crate::config::TracingConfig::default());
        assert_eq!(
            (tracing.filter.as_str(), tracing.format.as_str()),
            ("warn", "compact")
        );
        // the selector is the section: the module's start then asks for a
        // path and a realm (modules::oci::login_from_settings)
        let settings =
            load(None, env(&[("REGISTRY_AUTH", "htpasswd")])).expect("the guide's selector");
        assert_eq!(settings.get("auth.htpasswd"), Some("htpasswd"));
        // the reference's own file variable, and Kubernetes service links
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "http:\n  addr: :5000\n").expect("write");
        let settings = load(
            None,
            env(&[
                ("REGISTRY_CONFIGURATION_PATH", path.to_str().expect("utf-8")),
                ("REGISTRY_PORT", "tcp://10.0.0.1:5000"),
                ("REGISTRY_PORT_5000_TCP_ADDR", "10.0.0.1"),
                ("REGISTRY_SERVICE_HOST", "10.0.0.1"),
            ]),
        )
        .expect("load");
        assert_eq!(settings.get("http.addr"), Some(":5000"));
        assert_eq!(settings.skipped.len(), 3);
        // a header that cannot be sent stops the start
        assert!(load_text("http:\n  headers:\n    \"Bad Name\": [x]\n", &[]).is_err());
        // an empty section sets nothing, and yes is true
        let settings = load_text(
            "storage:\n  filesystem:\n  delete:\n    enabled: yes\n",
            &[],
        )
        .expect("load");
        assert_eq!(settings.flag("storage.delete.enabled"), Some(true));
    }

    #[test]
    fn refused_values() {
        assert!(load_text("log:\n  formatter: logstash\n", &[]).is_err());
        assert!(load_text("storage:\n  cache:\n    blobdescriptor: redis\n", &[]).is_err());
        assert!(load_text("log:\n  formatter: json\n", &[]).is_ok());
    }
}
