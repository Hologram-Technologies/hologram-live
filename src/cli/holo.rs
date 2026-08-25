use super::{helpers, run, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::{inspect_bytes, plan_bytes, HoloCatalog};
use hologram_live::protocol::{HoloInspection, RpcRequest, RpcResponse};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct HoloArgs {
    #[command(subcommand)]
    command: HoloCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum HoloCommand {
    Fixture {
        output: PathBuf,
    },
    Import {
        path: PathBuf,
    },
    List,
    Inspect {
        /// Catalog kappa, or a local .holo file.
        reference: String,
    },
    Plan {
        /// Catalog kappa, or a local .holo file.
        reference: String,
    },
    Verify {
        /// Catalog kappa, or a local .holo file.
        reference: String,
    },
    Load {
        kappa: String,
    },
    Unload {
        kappa: String,
    },
    Run {
        kappa: String,
        #[arg(long = "input")]
        inputs: Vec<PathBuf>,
        /// Render application outputs as raw protocol bytes, UTF-8 text, or JSON.
        #[arg(long, value_enum, default_value_t = run::RunOutputFormat::Raw)]
        output_format: run::RunOutputFormat,
    },
    Resident,
    Remove {
        kappa: String,
    },
}

pub async fn run(cli: Cli, args: HoloArgs) -> Result<()> {
    match args.command {
        HoloCommand::Fixture { output } => fixture(&cli, output).await,
        HoloCommand::Import { path } => import(&cli, path).await,
        HoloCommand::List => match helpers::call(&cli, RpcRequest::HoloList).await? {
            RpcResponse::HoloList(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        HoloCommand::Inspect { reference } => inspect(&cli, reference, false).await,
        HoloCommand::Plan { reference } => plan(&cli, reference).await,
        HoloCommand::Verify { reference } => inspect(&cli, reference, true).await,
        HoloCommand::Load { kappa } => {
            match helpers::call(&cli, RpcRequest::HoloLoad { kappa }).await? {
                RpcResponse::HoloResident(value) => helpers::print(&cli, &value),
                other => helpers::unexpected(other),
            }
        }
        HoloCommand::Unload { kappa } => helpers::expect_accepted(
            &cli,
            helpers::call(&cli, RpcRequest::HoloUnload { kappa }).await?,
        ),
        HoloCommand::Run {
            kappa,
            inputs,
            output_format,
        } => {
            run::run(
                cli,
                run::RunArgs {
                    reference: kappa,
                    inputs,
                    output_format,
                },
            )
            .await
        }
        HoloCommand::Resident => match helpers::call(&cli, RpcRequest::HoloResident).await? {
            RpcResponse::HoloResident(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        HoloCommand::Remove { kappa } => {
            let _ = helpers::call(
                &cli,
                RpcRequest::HoloUnload {
                    kappa: kappa.clone(),
                },
            )
            .await;
            helpers::expect_accepted(
                &cli,
                helpers::call(&cli, RpcRequest::HoloRemove { kappa }).await?,
            )
        }
    }
}

async fn fixture(cli: &Cli, output: PathBuf) -> Result<()> {
    let bytes = tokio::task::spawn_blocking(HoloCatalog::fixture)
        .await
        .map_err(|error| LiveError::Conflict(format!("fixture task failed: {error}")))??;
    let byte_length = bytes.len();
    tokio::fs::write(&output, &bytes)
        .await
        .map_err(|error| LiveError::io(&output, error))?;
    if cli.json {
        helpers::print(
            cli,
            &serde_json::json!({
                "status": "written",
                "output": output,
                "byte_length": byte_length
            }),
        )
    } else {
        helpers::message(
            cli,
            "written",
            format!(
                "wrote structurally valid .holo fixture to {}",
                output.display()
            ),
        )
    }
}

async fn import(cli: &Cli, path: PathBuf) -> Result<()> {
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| LiveError::io(&path, error))?;
    let name = path.file_name().map_or_else(
        || "application.holo".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    );
    match helpers::call(cli, RpcRequest::HoloImport { name, bytes }).await? {
        RpcResponse::HoloInspection(value) => helpers::print(cli, &value),
        other => helpers::unexpected(other),
    }
}

async fn info(cli: &Cli, request: RpcRequest) -> Result<()> {
    match helpers::call(cli, request).await? {
        RpcResponse::HoloInspection(value) => helpers::print(cli, &value),
        other => helpers::unexpected(other),
    }
}

async fn inspect(cli: &Cli, reference: String, verify: bool) -> Result<()> {
    if reference.starts_with("blake3:") {
        let request = if verify {
            RpcRequest::HoloVerify { kappa: reference }
        } else {
            RpcRequest::HoloInspect { kappa: reference }
        };
        return info(cli, request).await;
    }

    let inspection = inspect_local(PathBuf::from(reference)).await?;
    helpers::print(cli, &inspection)
}

async fn plan(cli: &Cli, reference: String) -> Result<()> {
    if reference.starts_with("blake3:") {
        return match helpers::call(cli, RpcRequest::HoloPlan { kappa: reference }).await? {
            RpcResponse::HoloPlan(value) => helpers::print(cli, &value),
            other => helpers::unexpected(other),
        };
    }

    let path = PathBuf::from(reference);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| LiveError::io(&path, error))?;
    helpers::print(cli, &plan_bytes(&bytes)?)
}

async fn inspect_local(path: PathBuf) -> Result<HoloInspection> {
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| LiveError::io(&path, error))?;
    let kappa = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    inspect_bytes(&kappa, &path.to_string_lossy(), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_file_inspection_returns_its_import_kappa() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("fixture.holo");
        let bytes = HoloCatalog::fixture().expect("fixture");
        tokio::fs::write(&path, &bytes)
            .await
            .expect("write fixture");

        let inspection = inspect_local(path.clone()).await.expect("inspect");

        assert_eq!(inspection.name, path.to_string_lossy());
        assert_eq!(
            inspection.kappa,
            format!("blake3:{}", blake3::hash(&bytes).to_hex())
        );
        assert!(inspection.footer_verified);
    }
}
