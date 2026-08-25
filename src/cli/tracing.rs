use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct TracingArgs {
    #[command(subcommand)]
    command: TracingCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum TracingCommand {
    Show,
    Set { filter: String },
}

pub async fn run(cli: Cli, args: TracingArgs) -> Result<()> {
    let request = match args.command {
        TracingCommand::Show => RpcRequest::TracingGet,
        TracingCommand::Set { filter } => RpcRequest::TracingSet { filter },
    };
    match helpers::call(&cli, request).await? {
        RpcResponse::TracingFilter(value) if cli.json => {
            helpers::print(&cli, &serde_json::json!({ "filter": value }))
        }
        RpcResponse::TracingFilter(value) => helpers::message(&cli, "ok", value),
        other => helpers::unexpected(other),
    }
}
