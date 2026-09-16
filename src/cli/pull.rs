use super::{helpers, Cli};
use clap::Args;
use hologram_live::artifact_pull::{pull as run_pull, LayerSource, PullProgress};
use hologram_live::artifact_ref::ArtifactRef;
use hologram_live::error::{LiveError, Result};
use hologram_live::registry::kappa_client::KappaClient;
use hologram_live::store::ObjectStore;

#[derive(Debug, Clone, Args)]
pub struct PullArgs {
    /// Artifact reference, for example `qwen3.5:4b` or
    /// `host:5000/models/qwen3.5:4b`.
    reference: String,
}

pub async fn run(cli: Cli, args: PullArgs) -> Result<()> {
    let (config, _) = helpers::load(&cli)?;
    let reference = ArtifactRef::parse(&args.reference, &config.registry)?;

    // Progress is inherently streaming and the result is a single document, so
    // they go to different streams. Under --json the progress is JSONL, so a
    // consumer can read events without a spinner corrupting stdout.
    let json = cli.json;
    let mut emit = move |progress: PullProgress| {
        if json {
            if let Ok(line) = serde_json::to_string(&progress) {
                eprintln!("{line}");
            }
        } else {
            let verb = match progress.source {
                LayerSource::AlreadyPresent => "present",
                LayerSource::Fetched => "fetched",
            };
            eprintln!(
                "{:>7} [{}/{}] {}",
                verb,
                progress.index + 1,
                progress.total,
                progress.kappa
            );
        }
    };

    // The client and store are built inside the blocking task, not before it.
    // `reqwest::blocking::Client` owns an internal runtime, and constructing
    // one from an async context panics when that runtime is dropped.
    let registry = config.registry.clone();
    let store_root = config.paths.data_dir.join("registry");
    let report = tokio::task::spawn_blocking(move || {
        let client = KappaClient::new(&registry)?;
        let store = ObjectStore::open(store_root)?;
        run_pull(&client, &store, &reference, &mut emit)
    })
    .await
    .map_err(|error| LiveError::Conflict(format!("join artifact pull: {error}")))??;

    helpers::print(&cli, &report)
}
