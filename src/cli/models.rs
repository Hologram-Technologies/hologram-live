use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    command: ModelsCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ModelsCommand {
    /// List imported inference models.
    List,
    /// Import a local .wcpu artifact directory produced by weightc.
    Import { path: PathBuf },
    /// Remove an imported model and its copied artifact.
    Remove { id: String },
}

pub async fn run(cli: Cli, args: ModelsArgs) -> Result<()> {
    match args.command {
        ModelsCommand::List => match helpers::call(&cli, RpcRequest::ModelList).await? {
            RpcResponse::Models(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        ModelsCommand::Import { path } => {
            // The daemon reads the artifact itself, so hand it an absolute path.
            let path = std::path::absolute(&path)?;
            let path = path.to_string_lossy().into_owned();
            match helpers::call(&cli, RpcRequest::ModelImport { path }).await? {
                RpcResponse::Model(value) => helpers::print(&cli, &value),
                other => helpers::unexpected(other),
            }
        }
        ModelsCommand::Remove { id } => {
            helpers::expect_accepted(helpers::call(&cli, RpcRequest::ModelRemove { id }).await?)
        }
    }
}
