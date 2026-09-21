use super::{helpers, Cli};
use clap::Args;
use hologram_live::app::AppState;
use hologram_live::error::Result;
use hologram_live::observability::TracingHandle;
use hologram_live::{process, server};

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    /// Artifact reference to make resident before the listener binds, for
    /// example `demo:v1`. A local path or `blake3:` kappa also works.
    reference: Option<String>,
    #[arg(long)]
    listen: Option<String>,
    /// Public origin this server advertises to cluster peers.
    #[arg(long, value_name = "URL")]
    advertise: Option<String>,
    /// Existing Hologram server to join. May be repeated.
    #[arg(long = "join", value_name = "URL")]
    join: Vec<String>,
}

/// Resolve a serve argument to a catalog kappa, acquiring it if needed.
///
/// Precedence matches `run`: a kappa is already catalogued, a path is a local
/// archive, and anything else is a registry reference. Importing is what makes
/// a pulled archive loadable -- `load_declared` resolves kappas through the
/// catalog, so a blob cached by the pull alone is not enough.
async fn resolve_resident(
    cli: &Cli,
    config: &hologram_live::config::AppConfig,
    reference: &str,
) -> Result<String> {
    use hologram_live::artifact_ref::{resolve, Resolution};
    use hologram_live::holo::HoloCatalog;
    use hologram_live::store::ObjectStore;
    use std::sync::Arc;

    let (bytes, name) = match resolve(reference, &config.registry)? {
        // Already catalogued: nothing to acquire or import.
        Resolution::Kappa(kappa) => return Ok(kappa),
        Resolution::File(path) => {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|error| hologram_live::error::LiveError::io(&path, error))?;
            let name = path.file_name().map_or_else(
                || "archive".to_owned(),
                |v| v.to_string_lossy().into_owned(),
            );
            (bytes, name)
        }
        Resolution::Reference(reference) => {
            let name = format!("{}:{}", reference.name, reference.tag);
            (pull_archive_bytes(cli, config, &reference).await?, name)
        }
    };

    let store = Arc::new(ObjectStore::open(config.paths.data_dir.join("registry"))?);
    let catalog = HoloCatalog::new(store);
    let inspection = tokio::task::spawn_blocking(move || catalog.import(name, bytes))
        .await
        .map_err(|error| {
            hologram_live::error::LiveError::Conflict(format!("join archive import: {error}"))
        })??;
    Ok(inspection.kappa)
}

/// Fetch a named artifact and return its archive bytes.
async fn pull_archive_bytes(
    cli: &Cli,
    config: &hologram_live::config::AppConfig,
    reference: &hologram_live::artifact_ref::ArtifactRef,
) -> Result<Vec<u8>> {
    use hologram_live::artifact_pull::{pull, LayerSource, PullProgress};
    use hologram_live::error::LiveError;
    use hologram_live::registry::kappa_client::KappaClient;
    use hologram_live::store::ObjectStore;

    let registry = config.registry.clone();
    let store_root = config.paths.data_dir.join("registry");
    let reference = reference.clone();
    let json = cli.json;
    let mut emit = move |progress: PullProgress| {
        if json {
            if let Ok(line) = serde_json::to_string(&progress) {
                eprintln!("{line}");
            }
        } else if progress.source == LayerSource::Fetched {
            eprintln!(
                "fetched [{}/{}] {}",
                progress.index + 1,
                progress.total,
                progress.kappa
            );
        }
    };

    tokio::task::spawn_blocking(move || {
        // Built inside the blocking task: `reqwest::blocking::Client` owns an
        // internal runtime, and constructing one from an async context panics
        // when that runtime is dropped.
        let client = KappaClient::new(&registry)?;
        let store = ObjectStore::open(store_root)?;
        let report = pull(&client, &store, &reference, &mut emit)?;
        store.get_cached(&report.archive_kappa)?.ok_or_else(|| {
            LiveError::NotFound(format!(
                "pulled archive {} is absent from the local store",
                report.archive_kappa
            ))
        })
    })
    .await
    .map_err(|error| {
        hologram_live::error::LiveError::Conflict(format!("join artifact pull: {error}"))
    })?
}

pub async fn run(cli: Cli, args: ServeArgs, tracing: TracingHandle) -> Result<()> {
    let (mut config, _) = helpers::load(&cli)?;
    let uses_default_cluster_endpoint = config.cluster.advertise_endpoint.as_deref()
        == Some(hologram_live::config::DEFAULT_CLUSTER_ENDPOINT);
    if let Some(listen) = args.listen {
        config.server.listen = listen;
        if args.advertise.is_none() && uses_default_cluster_endpoint {
            if let Ok(address) = config.server.listen.parse::<std::net::SocketAddr>() {
                if address.ip().is_loopback() {
                    config.cluster.advertise_endpoint = Some(format!("http://{address}"));
                }
            }
        }
    }
    if let Some(advertise) = args.advertise {
        config.cluster.advertise_endpoint = Some(advertise);
    }
    config.cluster.seeds.extend(args.join);
    config.validate()?;
    let listen = config.server.listen.clone();
    let mut declared: Vec<String> = config
        .holo
        .resident
        .iter()
        .map(|entry| entry.kappa.clone())
        .collect();
    // An argument joins the operator's declarations rather than replacing
    // them: `serve <ref>` is "also serve this", not "serve only this".
    if let Some(reference) = args.reference.as_deref() {
        declared.push(resolve_resident(&cli, &config, reference).await?);
    }
    let _guard = process::DaemonGuard::acquire(&config)?;
    let state = AppState::build(config, tracing.clone()).await?;
    // Load operator-declared resident applications before binding the
    // listener, so the daemon does not report ready until they are
    // invocable. Load time delays readiness probes; keep declarations
    // small. Failures skip only the failing entry and are already logged.
    if !declared.is_empty() {
        let outcomes = state.holo_runtime().load_declared(&declared).await;
        let loaded = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.is_ok())
            .count();
        tracing::info!(
            loaded,
            declared = outcomes.len(),
            "declared resident holo applications processed"
        );
    }
    let result = server::serve_with_ready(state, move || {
        if cli.json {
            helpers::print(
                &cli,
                &serde_json::json!({ "status": "serving", "listen": listen }),
            )
        } else {
            Ok(())
        }
    })
    .await;
    if let Err(error) = tracing.force_flush() {
        tracing::warn!(error = %error, "failed to flush telemetry during shutdown");
    }
    result
}
