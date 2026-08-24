use super::{helpers, Cli};
use clap::Args;
use hologram_live::error::{LiveError, Result};
use hologram_live::protocol::{RpcRequest, RpcResponse};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    pub(crate) reference: String,
    #[arg(long = "input")]
    pub(crate) inputs: Vec<PathBuf>,
}

pub async fn run(cli: Cli, args: RunArgs) -> Result<()> {
    let mut inputs = Vec::with_capacity(args.inputs.len());
    for path in args.inputs {
        inputs.push(
            tokio::fs::read(&path)
                .await
                .map_err(|error| LiveError::io(&path, error))?,
        );
    }
    match helpers::call(
        &cli,
        RpcRequest::HoloRun {
            kappa: args.reference,
            inputs,
        },
    )
    .await?
    {
        RpcResponse::HoloRun(value) => helpers::print(&cli, &value),
        other => helpers::unexpected(other),
    }
}
