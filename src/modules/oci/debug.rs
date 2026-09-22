//! The debug listener (`http.debug.addr`), as the reference runs it (FR-S09,
//! FR-R24): `/debug/health` for probes and load balancers, with
//! `/debug/health/down` and `/up` for a manual drain, and `/metrics` when
//! `http.debug.prometheus` is on. `/debug/vars` and pprof are Go internals and
//! answer 404 (D-008 in `impact-on-001.md`).
//!
//! Health is the reference's shape (docker/go-health): 200 `{}` when every
//! check passes, 503 with an object naming each failing check. The storage
//! check, `storagedriver_filesystem`, is a real write, read and delete on the
//! volume every `health.storagedriver.interval`, failing after `threshold`
//! failures in a row.

use crate::error::{LiveError, Result};
use crate::registry_compat::RegistrySettings;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The storage check's name, as the reference registers it for its driver.
const STORAGE_CHECK: &str = "storagedriver_filesystem";
/// The manual check's name and message, as go-health's `DownHandler` sets them.
const MANUAL_CHECK: &str = "manual_http_status";
const MANUAL_DOWN: &str = "Manual Check";

/// Failing checks: name → why.
static FAILING: Mutex<BTreeMap<&'static str, String>> = Mutex::new(BTreeMap::new());

/// Whether any check fails: the public port then answers 503 UNAVAILABLE,
/// as the reference's `health.Handler` does, which is what drains it.
pub fn failing() -> bool {
    !FAILING.lock().unwrap_or_else(PoisonError::into_inner).is_empty()
}

fn set(check: &'static str, failure: Option<String>) {
    let mut failing = FAILING.lock().unwrap_or_else(PoisonError::into_inner);
    match failure {
        Some(reason) => {
            failing.insert(check, reason);
        }
        None => {
            failing.remove(check);
        }
    }
}

/// What the registry's settings ask the debug listener to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugSettings {
    pub addr: String,
    /// `http.debug.prometheus.path`, when Prometheus is enabled.
    pub metrics: Option<String>,
    /// `health.storagedriver`, when enabled: (interval, threshold).
    pub storage: Option<(Duration, u32)>,
}

impl DebugSettings {
    /// From the registry's settings; `None` without `http.debug.addr`, as in
    /// the reference.
    ///
    /// # Errors
    ///
    /// An interval the reference would not read.
    pub fn from(settings: &RegistrySettings) -> Result<Option<Self>> {
        let Some(addr) = settings.get("http.debug.addr") else {
            return Ok(None);
        };
        let addr = match addr.trim().strip_prefix(':') {
            Some(port) => format!("0.0.0.0:{port}"),
            None => addr.trim().to_owned(),
        };
        let metrics = settings
            .flag("http.debug.prometheus.enabled")
            .unwrap_or(false)
            .then(|| settings.get("http.debug.prometheus.path").unwrap_or("/metrics").trim().to_owned());
        let storage = if settings.flag("health.storagedriver.enabled").unwrap_or(false) {
            let interval = match settings.get("health.storagedriver.interval") {
                Some(text) if text.trim() == "0" || text.trim() == "0s" => Duration::from_secs(10),
                Some(text) => go_duration(text).ok_or_else(|| {
                    LiveError::Config(format!("registry setting health.storagedriver.interval {text:?} is not a duration"))
                })?,
                None => Duration::from_secs(10),
            };
            let threshold = match settings.get("health.storagedriver.threshold") {
                Some(text) => text.trim().parse().map_err(|_| {
                    LiveError::Config(format!("registry setting health.storagedriver.threshold {text:?} is not a count"))
                })?,
                None => 1,
            };
            Some((interval, threshold))
        } else {
            None
        };
        if let Some(path) = &metrics {
            if !path.starts_with('/') || path.starts_with("/debug/health") {
                return Err(LiveError::Config(format!(
                    "registry setting http.debug.prometheus.path {path:?} must be a path of its own, starting with /"
                )));
            }
        }
        Ok(Some(Self { addr, metrics, storage }))
    }
}

/// A Go duration as the reference's configuration writes one: `10s`, `500ms`,
/// `1m30s`, or a bare number of nanoseconds.
pub(crate) fn go_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    if let Ok(nanos) = text.parse::<u64>() {
        return Some(Duration::from_nanos(nanos));
    }
    let mut total = 0.0_f64;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let value: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let (seconds, unit) = [("ns", 1e-9), ("us", 1e-6), ("µs", 1e-6), ("ms", 1e-3), ("s", 1.0), ("m", 60.0), ("h", 3600.0)]
            .into_iter()
            .find(|(unit, _)| rest.starts_with(unit))
            .map(|(unit, scale)| (value * scale, unit))?;
        total += seconds;
        rest = &rest[unit.len()..];
    }
    (total > 0.0).then(|| Duration::from_secs_f64(total))
}

/// The debug listener's routes.
pub fn router(settings: &DebugSettings) -> Router {
    let mut router = Router::new()
        .route("/debug/health", get(health))
        .route("/debug/health/down", post(down))
        .route("/debug/health/up", post(up));
    if let Some(path) = &settings.metrics {
        router = router.route(path, get(metrics));
    }
    router.fallback(|| async { (StatusCode::NOT_FOUND, "404 page not found\n") })
}

async fn health() -> Response {
    let failing = FAILING.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let status = if failing.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    // As the reference's statusResponse: json.Marshal, with no newline.
    let body = serde_json::to_string(&failing).unwrap_or_else(|_| "{}".to_owned());
    (status, [(header::CONTENT_TYPE, "application/json; charset=utf-8")], body).into_response()
}

async fn down() -> StatusCode {
    set(MANUAL_CHECK, Some(MANUAL_DOWN.to_owned()));
    StatusCode::OK
}

async fn up() -> StatusCode {
    set(MANUAL_CHECK, None);
    StatusCode::OK
}

async fn metrics() -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        super::metrics::render(),
    )
        .into_response()
}

/// The storage check: write, read back and delete a file on the volume.
fn probe(state_dir: &Path) -> std::result::Result<(), String> {
    use std::io::Write as _;
    // Every container's pid is 1: the name carries random bytes too.
    let mut nonce = [0_u8; 8];
    getrandom::fill(&mut nonce).map_err(|error| error.to_string())?;
    let path = state_dir.join(format!("health-probe-{}-{}", std::process::id(), crate::util::hex(&nonce)));
    let written = b"hologram registry health probe";
    std::fs::create_dir_all(state_dir).map_err(|error| error.to_string())?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(written))
        .map_err(|error| format!("write: {error}"))?;
    let read = std::fs::read(&path).map_err(|error| format!("read: {error}"));
    let removed = std::fs::remove_file(&path).map_err(|error| format!("delete: {error}"));
    match (read, removed) {
        (Ok(read), Ok(())) if read == written => Ok(()),
        (Ok(_), Ok(())) => Err("read back other bytes than it wrote".to_owned()),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

/// Run the storage check until the process ends.
pub fn watch_storage(state_dir: PathBuf, interval: Duration, threshold: u32) {
    tokio::spawn(async move {
        let mut failures = 0_u32;
        // The reference's ticker fires first after one interval.
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let dir = state_dir.clone();
            // A disk that hangs is a failing disk, not a healthy one.
            let result = match tokio::time::timeout(interval, tokio::task::spawn_blocking(move || probe(&dir))).await {
                Ok(joined) => joined.unwrap_or_else(|error| Err(error.to_string())),
                Err(_) => Err(format!("no answer within {} s", interval.as_secs())),
            };
            match result {
                Ok(()) => {
                    failures = 0;
                    set(STORAGE_CHECK, None);
                }
                Err(reason) => {
                    failures += 1;
                    tracing::warn!(failures, reason = reason.as_str(), "storage health check failed");
                    if failures >= threshold.max(1) {
                        set(STORAGE_CHECK, Some(reason));
                    }
                }
            }
        }
    });
}

/// Start the storage check the installed settings ask for. Called once the
/// volume's lock is held, so a second container refused the volume never
/// probes it.
pub fn start_checks(state_dir: &Path) {
    let Some(settings) = crate::registry_compat::installed() else {
        return;
    };
    if let Ok(Some(DebugSettings { storage: Some((interval, threshold)), .. })) = DebugSettings::from(settings) {
        watch_storage(state_dir.to_path_buf(), interval, threshold);
    }
}

/// Bind the debug listener the installed registry settings ask for, before
/// the public port: a port that cannot be bound stops the start.
///
/// # Errors
///
/// The settings are wrong, or the address cannot be bound.
pub async fn bind(public: &str) -> Result<Option<(tokio::net::TcpListener, Router)>> {
    let Some(settings) = crate::registry_compat::installed() else {
        return Ok(None);
    };
    let Some(wanted) = DebugSettings::from(settings)? else {
        return Ok(None);
    };
    // The image's default file puts the debug listener on :5001, and the
    // deployment guide moves the registry itself there with
    // REGISTRY_HTTP_ADDR=0.0.0.0:5001. The reference races its two binds and
    // one of them exits the process; here the registry keeps the port.
    if same_port(&wanted.addr, public) {
        tracing::warn!(
            debug = %wanted.addr,
            listen = public,
            "http.debug.addr is the registry's own port; the debug listener is not started"
        );
        return Ok(None);
    }
    let listener = tokio::net::TcpListener::bind(&wanted.addr)
        .await
        .map_err(|error| LiveError::Transport(format!("bind http.debug.addr {}: {error}", wanted.addr)))?;
    tracing::info!(listen = %wanted.addr, metrics = wanted.metrics.as_deref().unwrap_or("off"), "debug listener ready");
    Ok(Some((listener, router(&wanted))))
}

/// Whether two listen addresses would take the same port on this host.
fn same_port(a: &str, b: &str) -> bool {
    let port = |addr: &str| addr.rsplit_once(':').and_then(|(_, port)| port.parse::<u16>().ok());
    let host = |addr: &str| addr.rsplit_once(':').map(|(host, _)| host.trim_matches(['[', ']']).to_owned());
    let any = |host: &str| matches!(host, "" | "0.0.0.0" | "::");
    match (port(a), port(b), host(a), host(b)) {
        (Some(x), Some(y), Some(ha), Some(hb)) => x == y && (ha == hb || any(&ha) || any(&hb)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_debug_port_is_given_up_to_the_registry_itself() {
        assert!(same_port("0.0.0.0:5001", "0.0.0.0:5001"));
        assert!(same_port("0.0.0.0:5001", "127.0.0.1:5001"), "every interface includes loopback");
        assert!(!same_port("0.0.0.0:5001", "0.0.0.0:5000"));
        assert!(!same_port("127.0.0.1:5001", "10.0.0.2:5001"), "two hosts, two sockets");
    }

    #[test]
    fn go_durations_as_the_reference_writes_them() {
        assert_eq!(go_duration("10s"), Some(Duration::from_secs(10)));
        assert_eq!(go_duration("1m30s"), Some(Duration::from_secs(90)));
        assert_eq!(go_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(go_duration("1000000000"), Some(Duration::from_secs(1)));
        assert_eq!(go_duration("ten"), None);
        assert_eq!(go_duration("10"), Some(Duration::from_nanos(10)));
    }

    #[test]
    fn the_probe_writes_reads_and_deletes() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(probe(dir.path()), Ok(()));
        assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0, "nothing left behind");
    }
}
