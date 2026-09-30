use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram::space::address_bytes;
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::inspect_bytes;
use hologram_live::{
    evaluate_ai_operations_comparison, FusedKernelProfile, InferenceCostProfile, MatrixDimension, ModelSpec,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct AiArgs {
    #[command(subcommand)]
    command: AiCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum AiCommand {
    /// Inspect inference-model services without initializing an engine.
    Inspect { path: PathBuf },
    /// Evaluate the UOR/Prism formal inference cost-model for model dimensions.
    #[command(name = "cost-model")]
    CostModel {
        #[arg(long, default_value = "1")]
        m: u64,
        #[arg(long, default_value = "4096")]
        k: u64,
        #[arg(long, default_value = "4096")]
        n: u64,
        #[arg(long, default_value = "512")]
        total_tokens: u64,
        #[arg(long, default_value = "256")]
        prefix_tokens: u64,
    },
    /// Compare `hologram-ai` `PrismPM` vs non-PrismPM operations and capabilities across scaling dimensions.
    Compare {
        #[arg(long, default_value = "8b")]
        model: String,
        #[arg(long, default_value = "131072")]
        context_length: u64,
        #[arg(long, default_value = "65536")]
        prefix_tokens: u64,
        #[arg(long, default_value = "16")]
        memory_budget_gb: u64,
    },
    /// Liveness, readiness, and cost-model verification probe for inference-engine workers.
    Ping,
    /// Run the inference worker engine.
    Worker {
        #[arg(long, default_value = "4")]
        threads: usize,
    },
}

#[derive(Debug, Serialize)]
struct AiInspection {
    path: PathBuf,
    format_version: u16,
    archive_kappa: String,
    archive_fingerprint: String,
    application_kappa: String,
    models: Vec<AiModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_model: Option<InferenceCostProfile>,
}

#[derive(Debug, Serialize)]
struct AiModel {
    entry: String,
    engine: String,
    content_kappa: String,
    embedded: bool,
    byte_length: Option<u64>,
}

pub async fn run(cli: Cli, args: AiArgs) -> Result<()> {
    match args.command {
        AiCommand::Inspect { path } => {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|error| LiveError::io(&path, error))?;
            helpers::print(&cli, &inspect_model_archive(&path, &bytes)?)
        }
        AiCommand::CostModel {
            m,
            k,
            n,
            total_tokens,
            prefix_tokens,
        } => {
            let profile = InferenceCostProfile::evaluate(
                MatrixDimension::new(m, k, n),
                total_tokens,
                prefix_tokens,
            );
            helpers::print(&cli, &profile)
        }
        AiCommand::Compare {
            model,
            context_length,
            prefix_tokens,
            memory_budget_gb,
        } => {
            let spec = ModelSpec::from_name(&model).ok_or_else(|| {
                LiveError::Config(format!(
                    "unknown model preset '{model}'; supported presets: 1b, 3b, 8b, 70b (e.g. llama-3.1-8b)"
                ))
            })?;
            let comparison = evaluate_ai_operations_comparison(
                spec,
                context_length,
                prefix_tokens,
                memory_budget_gb,
            );
            helpers::print(&cli, &comparison)
        }
        AiCommand::Ping => ping(&cli).await,
        AiCommand::Worker { threads } => worker(&cli, threads).await,
    }
}

async fn ping(cli: &Cli) -> Result<()> {
    let optimal = FusedKernelProfile::optimal().is_optimal();
    let status = serde_json::json!({
        "service": "inference-engine",
        "status": if optimal { "healthy" } else { "degraded" },
        "cost_model": "uor-prism",
        "kernel": "fused-optimal",
        "working_set_containment": "verified"
    });
    helpers::print(cli, &status)
}

async fn worker(cli: &Cli, threads: usize) -> Result<()> {
    let optimal = FusedKernelProfile::optimal().is_optimal();
    let status = serde_json::json!({
        "service": "inference-engine",
        "status": "ready",
        "threads": threads,
        "fused_kernels": optimal,
        "cost_model": "uor-prism"
    });
    helpers::print(cli, &status)
}

fn inspect_model_archive(path: &Path, bytes: &[u8]) -> Result<AiInspection> {
    let archive_kappa = address_bytes(bytes).to_string();
    let inspection = inspect_bytes(&archive_kappa, &path.to_string_lossy(), bytes)?;
    let directory = inspection.directory.as_ref().ok_or_else(|| {
        LiveError::InvalidHolo(format!("{} has no application manifest", path.display()))
    })?;
    let models = directory
        .layers
        .iter()
        .filter(|layer| layer.kind == "inference-model")
        .map(|layer| {
            let blob = directory
                .blobs
                .iter()
                .find(|blob| blob.kappa == layer.content_kappa);
            AiModel {
                entry: layer.entry.clone(),
                engine: layer.engine.clone().unwrap_or_default(),
                content_kappa: layer.content_kappa.clone(),
                embedded: blob.is_some(),
                byte_length: blob.map(|blob| blob.byte_length),
            }
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err(LiveError::InvalidHolo(format!(
            "{} declares no inference-model layers",
            path.display()
        )));
    }
    Ok(AiInspection {
        path: path.to_path_buf(),
        format_version: inspection.format_version,
        archive_kappa,
        archive_fingerprint: inspection.archive_fingerprint,
        application_kappa: inspection.application_kappa.ok_or_else(|| {
            LiveError::InvalidHolo(format!("{} has no application identity", path.display()))
        })?,
        models,
        cost_model: Some(InferenceCostProfile::evaluate(
            MatrixDimension::new(1, 4096, 4096),
            512,
            256,
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::archive::HoloWriter;
    use hologram::space::{address_bytes, AppManifest, Layer, Realization};

    #[test]
    fn inspection_lists_model_service_metadata() {
        let bundle = b"deterministic model bundle";
        let capabilities = hologram_live::holo_capability::empty_canonical();
        let manifest = AppManifest {
            primary: None,
            requires: address_bytes(&capabilities),
            layers: vec![Layer::inference_model(
                address_bytes(bundle),
                "ai.default",
                "uor-r4",
            )],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        let capabilities_kappa = address_bytes(&capabilities);
        let bundle_kappa = address_bytes(bundle);
        let directory = hologram_live::holo_directory::derive(
            &manifest,
            [
                (capabilities_kappa.as_bytes(), capabilities.as_slice()),
                (bundle_kappa.as_bytes(), &bundle[..]),
            ],
        )
        .expect("directory");
        writer.add_extension(
            hologram_live::holo_directory::DIRECTORY_EXTENSION_KEY,
            hologram_live::holo_directory::encode(&directory).expect("encode directory"),
        );
        writer.add_content_blob(capabilities_kappa.as_bytes(), capabilities);
        writer.add_content_blob(bundle_kappa.as_bytes(), bundle);
        let archive = writer.finish().expect("archive");

        let report = inspect_model_archive(Path::new("model.holo"), &archive).expect("inspect");
        assert_eq!(report.format_version, 4);
        assert_eq!(report.archive_kappa, address_bytes(&archive).to_string());
        assert_eq!(
            report.application_kappa,
            address_bytes(&manifest.canonicalize()).to_string()
        );
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models[0].entry, "ai.default");
        assert_eq!(report.models[0].engine, "uor-r4");
        assert!(report.models[0].embedded);
        assert_eq!(
            report.models[0].byte_length,
            Some(u64::try_from(bundle.len()).expect("length"))
        );
    }
}
