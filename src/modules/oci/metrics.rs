//! `/metrics` on the debug listener (FR-S09, `operations.md` section 3), in the
//! Prometheus text format, under the reference's `registry_http_*` names so
//! existing dashboards draw.
//!
//! The `handler` label is the route's name, never the raw path: repository
//! names must not become label values.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

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
