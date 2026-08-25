use super::{helpers, Cli};
use clap::Args;
use hologram_live::compile::{compile_manifest_with, HoloPackaging};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::inspect_bytes;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct CompileArgs {
    /// JSON application manifest to compile.
    pub manifest: PathBuf,
    /// Destination archive. Defaults to the manifest path with a .holo suffix.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Emit a manifest-only archive whose layer payloads resolve by kappa.
    #[arg(long)]
    pub thin: bool,
}

#[derive(Debug, Serialize)]
struct CompileReport {
    output: PathBuf,
    layer_count: usize,
    byte_length: u64,
    archive_fingerprint: String,
    packaging: &'static str,
}

pub async fn run(cli: Cli, args: CompileArgs) -> Result<()> {
    let manifest = args.manifest.clone();
    let packaging = if args.thin {
        HoloPackaging::Thin
    } else {
        HoloPackaging::Fat
    };
    let compiled = tokio::task::spawn_blocking(move || compile_manifest_with(&manifest, packaging))
        .await
        .map_err(|error| LiveError::Conflict(format!("compile task failed: {error}")))??;
    let output = args
        .output
        .unwrap_or_else(|| args.manifest.with_extension("holo"));
    let inspection = inspect_bytes("local", &output.to_string_lossy(), &compiled.bytes)?;
    tokio::fs::write(&output, &compiled.bytes)
        .await
        .map_err(|error| LiveError::io(&output, error))?;
    helpers::print(
        &cli,
        &CompileReport {
            output,
            layer_count: compiled.layer_count,
            byte_length: inspection.byte_length,
            archive_fingerprint: inspection.archive_fingerprint,
            packaging: match compiled.packaging {
                HoloPackaging::Fat => "fat",
                HoloPackaging::Thin => "thin",
            },
        },
    )
}
