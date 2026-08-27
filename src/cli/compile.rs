use super::{helpers, Cli};
use clap::Args;
use hologram_live::compile::{
    check_manifest, compile_manifest_with_options, BuildProvenanceReport, CompileOptions,
    HoloPackaging,
};
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
    /// Validate the manifest and its source inputs without writing an archive.
    #[arg(long)]
    pub check: bool,
    /// Ignore reusable source-builder caches for this compilation.
    #[arg(long)]
    pub no_build_cache: bool,
}

#[derive(Debug, Serialize)]
struct CompileReport {
    output: PathBuf,
    layer_count: usize,
    child_count: usize,
    byte_length: u64,
    archive_kappa: String,
    archive_fingerprint: String,
    application_kappa: String,
    capabilities_kappa: String,
    packaging: &'static str,
    build_provenance: BuildProvenanceReport,
}

#[derive(Debug, Serialize)]
struct CheckReport {
    manifest: PathBuf,
    layer_count: usize,
    child_count: usize,
    schema_version: u16,
    capabilities_kappa: String,
    build_provenance: BuildProvenanceReport,
    valid: bool,
}

pub async fn run(cli: Cli, args: CompileArgs) -> Result<()> {
    if args.check {
        if args.output.is_some() || args.thin || args.no_build_cache {
            return Err(LiveError::Config(
                "compile --check cannot be combined with --output, --thin, or --no-build-cache"
                    .to_owned(),
            ));
        }
        let manifest = args.manifest.clone();
        let checked = tokio::task::spawn_blocking(move || check_manifest(&manifest))
            .await
            .map_err(|error| LiveError::Conflict(format!("check task failed: {error}")))??;
        return helpers::print(
            &cli,
            &CheckReport {
                manifest: args.manifest,
                layer_count: checked.specification.layers.len(),
                child_count: checked.specification.children.len(),
                schema_version: checked.specification.schema_version,
                capabilities_kappa: checked.capabilities_kappa,
                build_provenance: checked.build_provenance,
                valid: true,
            },
        );
    }
    let manifest = args.manifest.clone();
    let packaging = if args.thin {
        HoloPackaging::Thin
    } else {
        HoloPackaging::Fat
    };
    let options = CompileOptions {
        no_build_cache: args.no_build_cache,
    };
    let compiled = tokio::task::spawn_blocking(move || {
        compile_manifest_with_options(&manifest, packaging, options)
    })
    .await
    .map_err(|error| LiveError::Conflict(format!("compile task failed: {error}")))??;
    let output = args
        .output
        .unwrap_or_else(|| args.manifest.with_extension("holo"));
    let inspection = inspect_bytes(
        &compiled.identity.archive_kappa,
        &output.to_string_lossy(),
        &compiled.bytes,
    )?;
    tokio::fs::write(&output, &compiled.bytes)
        .await
        .map_err(|error| LiveError::io(&output, error))?;
    helpers::print(
        &cli,
        &CompileReport {
            output,
            layer_count: compiled.layer_count,
            child_count: compiled.child_count,
            byte_length: inspection.byte_length,
            archive_kappa: compiled.identity.archive_kappa,
            archive_fingerprint: compiled.identity.archive_fingerprint,
            application_kappa: compiled.identity.application_kappa,
            capabilities_kappa: compiled.capabilities_kappa,
            packaging: match compiled.packaging {
                HoloPackaging::Fat => "fat",
                HoloPackaging::Thin => "thin",
            },
            build_provenance: compiled.build_provenance,
        },
    )
}
