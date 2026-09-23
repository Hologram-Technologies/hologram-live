//! `/metrics` on the debug listener (FR-S09, `operations.md` section 3), in the
//! Prometheus text format, under the reference's `registry_http_*` names so
//! existing dashboards draw.
//!
//! The `handler` label is the route's name, never the raw path: repository
//! names must not become label values.

use crate::oci_store::OciStore;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

/// How often the volume gauges are read again (`operations.md` section 3).
const REFRESH: Duration = Duration::from_mins(1);

/// Bytes the registry took in through uploads, and served from blob `GET`s.
static UPLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
static SERVED_BYTES: AtomicU64 = AtomicU64::new(0);
/// Pushes whose bytes did not hash to the digest given.
static MISMATCHES: AtomicU64 = AtomicU64::new(0);

/// The volume, as last read: set by [`watch_store`].
struct Volume {
    store: Weak<OciStore>,
    path: String,
    reading: Mutex<Option<(crate::oci_store::Usage, u64)>>,
}

static VOLUME: OnceLock<Volume> = OnceLock::new();

/// `n` more bytes taken in by an upload.
pub fn uploaded(n: u64) {
    UPLOAD_BYTES.fetch_add(n, Ordering::Relaxed);
}

/// `n` more bytes of a blob sent.
pub fn served(n: u64) {
    SERVED_BYTES.fetch_add(n, Ordering::Relaxed);
}

/// Count a push the store refused because its bytes did not match its digest.
pub fn count_mismatch(error: &crate::oci_store::OciStoreError) {
    if matches!(error, crate::oci_store::OciStoreError::DigestMismatch { .. }) {
        MISMATCHES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Read the volume now and then every minute, on the blocking pool, until the
/// store is dropped: the store gauges and the free space of `path`.
pub fn watch_store(store: Weak<OciStore>, path: PathBuf) {
    let volume = VOLUME.get_or_init(|| Volume {
        store: store.clone(),
        path: path.display().to_string(),
        reading: Mutex::new(None),
    });
    let mut ticks = tokio::time::interval(REFRESH);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tokio::spawn(async move {
        loop {
            ticks.tick().await;
            let Some(store) = volume.store.upgrade() else { return };
            let path = path.clone();
            let read = tokio::task::spawn_blocking(move || {
                let usage = store.usage()?;
                let free = fs4::available_space(&path)
                    .map_err(|error| crate::oci_store::OciStoreError::Io(format!("free space of {}: {error}", path.display())))?;
                Ok::<_, crate::oci_store::OciStoreError>((usage, free))
            })
            .await;
            match read {
                Ok(Ok(reading)) => {
                    *volume.reading.lock().unwrap_or_else(PoisonError::into_inner) = Some(reading);
                }
                Ok(Err(error)) => tracing::warn!(%error, "the volume gauges were not read"),
                Err(_) => return,
            }
        }
    });
}

/// Prometheus quotes `\`, `"` and newlines in a label value.
fn label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// The default Prometheus buckets, in seconds.
const BUCKETS: [f64; 11] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

#[derive(Default)]
struct Histogram {
    buckets: [u64; BUCKETS.len()],
    count: u64,
    sum: f64,
}

#[derive(Default)]
struct Metrics {
    /// (handler, method, code) → requests.
    requests: BTreeMap<(&'static str, &'static str, u16), u64>,
    /// (handler, method) → durations.
    durations: BTreeMap<(&'static str, &'static str), Histogram>,
    /// handler → requests being answered now.
    in_flight: BTreeMap<&'static str, i64>,
}

static METRICS: Mutex<Option<Metrics>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Metrics) -> T) -> T {
    let mut guard = METRICS.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(Metrics::default))
}

/// A request under way: counted in flight until [`InFlight::done`], or until
/// it is dropped (the client went away): the gauge never leaks.
pub struct InFlight {
    handler: &'static str,
    method: &'static str,
    started: std::time::Instant,
    open: bool,
}

/// A method label from a fixed set: an unauthenticated client must not be
/// able to grow the label space.
fn method_label(method: &axum::http::Method) -> &'static str {
    match *method {
        axum::http::Method::GET => "get",
        axum::http::Method::HEAD => "head",
        axum::http::Method::POST => "post",
        axum::http::Method::PUT => "put",
        axum::http::Method::PATCH => "patch",
        axum::http::Method::DELETE => "delete",
        axum::http::Method::OPTIONS => "options",
        _ => "other",
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if self.open {
            with(|metrics| *metrics.in_flight.entry(self.handler).or_default() -= 1);
        }
    }
}

/// Start counting a request to `handler`.
pub fn start(handler: &'static str, method: &axum::http::Method) -> InFlight {
    with(|metrics| *metrics.in_flight.entry(handler).or_default() += 1);
    InFlight {
        handler,
        method: method_label(method),
        started: std::time::Instant::now(),
        open: true,
    }
}

impl InFlight {
    /// The request was answered with `status`.
    pub fn done(mut self, status: u16) {
        let elapsed = self.started.elapsed();
        self.open = false;
        with(|metrics| {
            *metrics.in_flight.entry(self.handler).or_default() -= 1;
            *metrics
                .requests
                .entry((self.handler, self.method, status))
                .or_default() += 1;
            let histogram = metrics.durations.entry((self.handler, self.method)).or_default();
            observe(histogram, elapsed);
        });
    }
}

fn observe(histogram: &mut Histogram, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    for (bucket, bound) in histogram.buckets.iter_mut().zip(BUCKETS) {
        if seconds <= bound {
            *bucket += 1;
        }
    }
    histogram.count += 1;
    histogram.sum += seconds;
}

/// The whole exposition.
pub fn render() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# HELP hologram_build_info The running build.");
    let _ = writeln!(out, "# TYPE hologram_build_info gauge");
    let _ = writeln!(
        out,
        "hologram_build_info{{features=\"oci\",version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    );
    with(|metrics| {
        let _ = writeln!(out, "# HELP registry_http_requests_total Requests answered, by route, method and status.");
        let _ = writeln!(out, "# TYPE registry_http_requests_total counter");
        for ((handler, method, code), count) in &metrics.requests {
            let _ = writeln!(
                out,
                "registry_http_requests_total{{code=\"{code}\",handler=\"{handler}\",method=\"{method}\"}} {count}"
            );
        }
        let _ = writeln!(out, "# HELP registry_http_in_flight_requests Requests being answered now, by route.");
        let _ = writeln!(out, "# TYPE registry_http_in_flight_requests gauge");
        for (handler, count) in &metrics.in_flight {
            let _ = writeln!(out, "registry_http_in_flight_requests{{handler=\"{handler}\"}} {count}");
        }
        let _ = writeln!(out, "# HELP registry_http_request_duration_seconds Time to answer, by route and method.");
        let _ = writeln!(out, "# TYPE registry_http_request_duration_seconds histogram");
        for ((handler, method), histogram) in &metrics.durations {
            let labels = format!("handler=\"{handler}\",method=\"{method}\"");
            for (bound, count) in BUCKETS.iter().zip(histogram.buckets) {
                let _ = writeln!(
                    out,
                    "registry_http_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {count}"
                );
            }
            let _ = writeln!(
                out,
                "registry_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}",
                histogram.count
            );
            let _ = writeln!(out, "registry_http_request_duration_seconds_sum{{{labels}}} {}", histogram.sum);
            let _ = writeln!(out, "registry_http_request_duration_seconds_count{{{labels}}} {}", histogram.count);
        }
    });
    let counters = [
        ("hologram_oci_upload_bytes_total", "Bytes taken in by blob uploads.", &UPLOAD_BYTES),
        ("hologram_oci_blob_bytes_served_total", "Bytes of blobs sent.", &SERVED_BYTES),
        ("hologram_oci_digest_mismatch_total", "Pushes refused because the bytes did not match the digest.", &MISMATCHES),
    ];
    for (name, help, value) in counters {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} counter");
        let _ = writeln!(out, "{name} {}", value.load(Ordering::Relaxed));
    }
    if let Some(volume) = VOLUME.get() {
        if let Some(store) = volume.store.upgrade() {
            let _ = writeln!(out, "# HELP hologram_oci_uploads_in_progress Upload sessions open now.");
            let _ = writeln!(out, "# TYPE hologram_oci_uploads_in_progress gauge");
            let _ = writeln!(out, "hologram_oci_uploads_in_progress {}", store.uploads_in_progress());
        }
        let reading = *volume.reading.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((usage, free)) = reading {
            let gauges = [
                ("hologram_oci_store_blobs", "Objects the registry stored, read once a minute.", usage.objects),
                ("hologram_oci_store_bytes", "Bytes under the blob root, read once a minute.", usage.bytes),
                ("hologram_oci_repositories", "Repositories, read once a minute.", usage.repositories),
            ];
            for (name, help, value) in gauges {
                let _ = writeln!(out, "# HELP {name} {help}");
                let _ = writeln!(out, "# TYPE {name} gauge");
                let _ = writeln!(out, "{name} {value}");
            }
            let _ = writeln!(out, "# HELP hologram_disk_free_bytes Bytes free to this process on the volume, read once a minute.");
            let _ = writeln!(out, "# TYPE hologram_disk_free_bytes gauge");
            let _ = writeln!(out, "hologram_disk_free_bytes{{path=\"{}\"}} {free}", label(&volume.path));
        }
    }
    out.push_str(&crate::tls::metrics_text());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_counted_by_route_method_and_status() {
        let request = start("manifest", &axum::http::Method::GET);
        assert!(render().contains("registry_http_in_flight_requests{handler=\"manifest\"} 1"));
        request.done(200);
        let text = render();
        assert!(text.contains("registry_http_requests_total{code=\"200\",handler=\"manifest\",method=\"get\"} 1"), "{text}");
        assert!(text.contains("registry_http_in_flight_requests{handler=\"manifest\"} 0"), "{text}");
        assert!(text.contains("registry_http_request_duration_seconds_count{handler=\"manifest\",method=\"get\"} 1"), "{text}");
        assert!(text.contains("le=\"+Inf\"} 1"), "{text}");
        assert!(text.contains("hologram_build_info{"), "{text}");
    }
}
