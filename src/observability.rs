use crate::config::{TelemetryConfig, TracingConfig};
use crate::error::{LiveError, Result};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Registry;

#[derive(Clone)]
pub struct TracingHandle {
    reload: reload::Handle<EnvFilter, Registry>,
    current: Arc<RwLock<String>>,
    providers: Arc<TelemetryProviders>,
}

struct TelemetryProviders {
    tracer: Option<SdkTracerProvider>,
    meter: Option<SdkMeterProvider>,
}

impl TracingHandle {
    pub fn current_filter(&self) -> Result<String> {
        self.current
            .read()
            .map(|filter| filter.clone())
            .map_err(|_| LiveError::Conflict("tracing filter lock poisoned".to_owned()))
    }

    pub fn set_filter(&self, value: &str) -> Result<()> {
        self.reload
            .reload(parse_filter(value)?)
            .map_err(|error| LiveError::Config(format!("reload tracing filter: {error}")))?;
        let mut current = self
            .current
            .write()
            .map_err(|_| LiveError::Conflict("tracing filter lock poisoned".to_owned()))?;
        value.clone_into(&mut *current);
        Ok(())
    }

    pub fn telemetry_exporting(&self) -> bool {
        self.providers.tracer.is_some() || self.providers.meter.is_some()
    }

    pub fn force_flush(&self) -> Result<()> {
        if let Some(provider) = &self.providers.tracer {
            provider
                .force_flush()
                .map_err(|error| LiveError::Transport(format!("flush OTLP traces: {error}")))?;
        }
        if let Some(provider) = &self.providers.meter {
            provider
                .force_flush()
                .map_err(|error| LiveError::Transport(format!("flush OTLP metrics: {error}")))?;
        }
        Ok(())
    }
}

pub fn init(tracing: &TracingConfig, telemetry: &TelemetryConfig) -> Result<TracingHandle> {
    let filter = parse_filter(&tracing.filter)?;
    let (filter_layer, reload) = reload::Layer::new(filter);
    let current = Arc::new(RwLock::new(tracing.filter.clone()));
    let (providers, tracer) = build_telemetry(telemetry)?;

    if let Some(tracer) = tracer {
        match tracing.format.as_str() {
            "json" => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .json()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
            "compact" => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .compact()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
            _ => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .pretty()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
        }
    } else {
        match tracing.format.as_str() {
            "json" => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .json()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .try_init(),
            "compact" => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .compact()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .try_init(),
            _ => tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .pretty()
                        .with_target(tracing.include_target)
                        .with_thread_ids(tracing.include_thread_ids),
                )
                .try_init(),
        }
    }
    .map_err(|error| LiveError::Config(format!("initialize tracing: {error}")))?;

    Ok(TracingHandle {
        reload,
        current,
        providers: Arc::new(providers),
    })
}

/// Test-only: a [`TracingHandle`] safe to obtain from more than one test in
/// the same binary.
///
/// `init` calls `tracing_subscriber`'s global `try_init`, which returns
/// `Err` — not panic — once a subscriber has already been installed in this
/// process. `AppState::build` requires a `TracingHandle`, so any test that
/// builds a real `AppState` (there is no lighter constructor) calls `init`
/// on its way there; a second such test in the same binary would get that
/// `Err`, and `.expect()`ing it would panic on whichever caller loses the
/// race — `cargo test` runs a binary's tests on multiple threads by
/// default, so which caller that is depends on scheduling.
///
/// Mirrors [`crate::util::install_crypto_provider`]'s `Once` pattern:
/// installs the global subscriber at most once, process-wide, and hands
/// every caller — including the first — the same [`TracingHandle`] (it is
/// [`Clone`]) rather than a fresh attempt that could fail. Production
/// startup (`src/cli/serve.rs`) calls [`init`] directly, never this
/// wrapper, so a genuine failure to install tracing there is untouched and
/// still propagates as an error on first use.
#[cfg(test)]
pub(crate) fn init_for_test(
    tracing: &TracingConfig,
    telemetry: &TelemetryConfig,
) -> Result<TracingHandle> {
    static HANDLE: std::sync::OnceLock<std::result::Result<TracingHandle, String>> =
        std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| init(tracing, telemetry).map_err(|error| error.to_string()))
        .clone()
        .map_err(LiveError::Config)
}

fn build_telemetry(
    config: &TelemetryConfig,
) -> Result<(
    TelemetryProviders,
    Option<opentelemetry_sdk::trace::SdkTracer>,
)> {
    let Some(endpoint) = config.endpoint.as_deref().filter(|_| config.enabled) else {
        return Ok((
            TelemetryProviders {
                tracer: None,
                meter: None,
            },
            None,
        ));
    };
    let timeout = Duration::from_secs(config.export_timeout_secs);
    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(timeout)
        .build()
        .map_err(|error| LiveError::Config(format!("configure OTLP trace exporter: {error}")))?;
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();
    let tracer = tracer_provider.tracer("hologram-live");
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(timeout)
        .build()
        .map_err(|error| LiveError::Config(format!("configure OTLP metric exporter: {error}")))?;
    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_periodic_exporter(metric_exporter)
        .build();
    opentelemetry::global::set_meter_provider(meter_provider.clone());

    Ok((
        TelemetryProviders {
            tracer: Some(tracer_provider),
            meter: Some(meter_provider),
        },
        Some(tracer),
    ))
}

fn parse_filter(value: &str) -> Result<EnvFilter> {
    EnvFilter::builder()
        .with_regex(false)
        .parse(value)
        .map_err(|error| LiveError::Config(format!("invalid tracing filter: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fix round 2 on the cluster replication resilience work:
    /// `AppState::build` requires a `TracingHandle`, and `init`'s global
    /// `try_init` errors on a second call in the same process, so any two
    /// tests in this binary that each build a real `AppState` would
    /// otherwise race for that one slot — `cargo test` runs a binary's
    /// tests on multiple threads by default, so `.expect()`ing `init`
    /// directly would panic on whichever caller loses. Pins that
    /// `init_for_test` survives exactly that shape of concurrent access:
    /// every thread gets a usable handle, not just the one that happens to
    /// install first.
    #[test]
    fn obtaining_the_test_tracing_handle_is_idempotent_across_threads() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    init_for_test(&TracingConfig::default(), &TelemetryConfig::default())
                })
            })
            .collect();
        for thread in threads {
            thread
                .join()
                .expect("obtaining the handle must not panic")
                .expect("every caller must get a usable TracingHandle");
        }
    }
}
