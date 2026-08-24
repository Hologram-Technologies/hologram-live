use super::{helpers, run, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::HoloCatalog;
use hologram_live::protocol::{RpcRequest, RpcResponse};
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
        kappa: String,
    },
    Verify {
        kappa: String,
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
    },
    Resident,
    Remove {
        kappa: String,
    },
}

pub async fn run(cli: Cli, args: HoloArgs) -> Result<()> {
    match args.command {
        HoloCommand::Fixture { output } => fixture(output).await,
        HoloCommand::Import { path } => import(&cli, path).await,
        HoloCommand::List => match helpers::call(&cli, RpcRequest::HoloList).await? {
            RpcResponse::HoloList(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        HoloCommand::Inspect { kappa } => info(&cli, RpcRequest::HoloInspect { kappa }).await,
        HoloCommand::Verify { kappa } => info(&cli, RpcRequest::HoloVerify { kappa }).await,
        HoloCommand::Load { kappa } => {
            match helpers::call(&cli, RpcRequest::HoloLoad { kappa }).await? {
                RpcResponse::HoloResident(value) => helpers::print(&cli, &value),
                other => helpers::unexpected(other),
            }
        }
        HoloCommand::Unload { kappa } => {
            helpers::expect_accepted(helpers::call(&cli, RpcRequest::HoloUnload { kappa }).await?)
        }
        HoloCommand::Run { kappa, inputs } => {
            run::run(
                cli,
                run::RunArgs {
                    reference: kappa,
                    inputs,
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
            helpers::expect_accepted(helpers::call(&cli, RpcRequest::HoloRemove { kappa }).await?)
        }
    }
}

async fn fixture(output: PathBuf) -> Result<()> {
    let bytes = tokio::task::spawn_blocking(HoloCatalog::fixture)
        .await
        .map_err(|error| LiveError::Conflict(format!("fixture task failed: {error}")))??;
    tokio::fs::write(&output, bytes)
        .await
        .map_err(|error| LiveError::io(&output, error))?;
    println!(
        "wrote structurally valid .holo fixture to {}",
        output.display()
    );
    Ok(())
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
