use super::{helpers, Cli};
use clap::Args;
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::HoloExecutor;
use hologram_live::protocol::{RpcRequest, RpcResponse};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Catalog kappa, or a local self-contained .holo file.
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
    let local = PathBuf::from(&args.reference);
    if local.is_file()
        || local
            .extension()
            .is_some_and(|extension| extension == "holo")
    {
        let bytes = tokio::fs::read(&local)
            .await
            .map_err(|error| LiveError::io(&local, error))?;
        let result =
            tokio::task::spawn_blocking(move || HoloExecutor::default().execute(&bytes, inputs))
                .await
                .map_err(|error| {
                    LiveError::Conflict(format!("local holo execution task failed: {error}"))
                })??;
        return helpers::print(&cli, &result);
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
