use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    command: RegistryCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum RegistryCommand {
    List,
}

pub async fn run(cli: Cli, args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommand::List => match helpers::call(&cli, RpcRequest::RegistryList).await? {
            RpcResponse::Objects(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
    }
}
