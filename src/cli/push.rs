use super::{helpers, Cli};
use clap::Args;
use hologram_live::artifact_push::{push as run_push, PushProgress};
use hologram_live::artifact_ref::ArtifactRef;
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::inspect_bytes;
use hologram_live::registry::kappa_client::KappaClient;
use hologram_live::store::ObjectStore;
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct PushArgs {
    /// Local `.holo` archive to publish.
    archive: PathBuf,
    /// Target reference, for example `demo:v1`.
    reference: String,
    /// Move the tag even if it already resolves to something else.
    #[arg(long)]
    force: bool,
}

pub async fn run(cli: Cli, args: PushArgs) -> Result<()> {
    let (config, _) = helpers::load(&cli)?;
    let reference = ArtifactRef::parse(&args.reference, &config.registry)?;

    let bytes = tokio::fs::read(&args.archive)
        .await
        .map_err(|error| LiveError::io(&args.archive, error))?;
    let name = args.archive.file_name().map_or_else(
        || "archive".to_owned(),
        |v| v.to_string_lossy().into_owned(),
    );
    let kappa = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    // Inspecting before publishing serves two purposes: it rejects a file that
    // is not a valid archive before anything reaches the registry, and it is
    // how the referenced-but-not-embedded payloads are enumerated.
    let inspection = inspect_bytes(&kappa, &name, &bytes)?;

    let json = cli.json;
    let mut emit = move |progress: PushProgress| {
        if json {
            if let Ok(line) = serde_json::to_string(&progress) {
                eprintln!("{line}");
            }
        } else {
            eprintln!(
                "published [{}/{}] {}",
                progress.index + 1,
                progress.total,
                progress.kappa
            );
        }
    };

    let registry = config.registry.clone();
    let store_root = config.paths.data_dir.join("registry");
    let force = args.force;
    let report = tokio::task::spawn_blocking(move || {
        // Built inside the blocking task: `reqwest::blocking::Client` owns an
        // internal runtime, and constructing one from an async context panics
        // when that runtime is dropped.
        let client = KappaClient::new(&registry)?;
        let store = ObjectStore::open(store_root)?;
        run_push(
            &client,
            &store,
            &reference,
            &bytes,
            &inspection,
            force,
            &mut emit,
        )
    })
    .await
    .map_err(|error| LiveError::Conflict(format!("join artifact push: {error}")))??;

    helpers::print(&cli, &report)
}
