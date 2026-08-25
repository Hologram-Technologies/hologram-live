use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::inspect_bytes;
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
}

#[derive(Debug, Serialize)]
struct AiInspection {
    path: PathBuf,
    format_version: u16,
    archive_fingerprint: String,
    models: Vec<AiModel>,
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
    }
}

fn inspect_model_archive(path: &Path, bytes: &[u8]) -> Result<AiInspection> {
    let inspection = inspect_bytes("local", &path.to_string_lossy(), bytes)?;
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
        archive_fingerprint: inspection.archive_fingerprint,
        models,
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
        let manifest = AppManifest {
            primary: None,
            requires: address_bytes(&[]),
            layers: vec![Layer::inference_model(
                address_bytes(bundle),
                "ai.default",
                "uor-r4",
            )],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(bundle).as_bytes(), bundle);
        let archive = writer.finish().expect("archive");

        let report = inspect_model_archive(Path::new("model.holo"), &archive).expect("inspect");
        assert_eq!(report.format_version, 4);
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
