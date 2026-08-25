use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{NodeRecord, RpcRequest, RpcResponse};
use hologram_live::util;

#[derive(Debug, Clone, Args)]
pub struct NodesArgs {
    #[command(subcommand)]
    command: NodesCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum NodesCommand {
    List,
    Heartbeat { node_id: String },
}

pub async fn run(cli: Cli, args: NodesArgs) -> Result<()> {
    match args.command {
        NodesCommand::List => match helpers::call(&cli, RpcRequest::NodesList).await? {
            RpcResponse::Nodes(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        NodesCommand::Heartbeat { node_id } => heartbeat(&cli, node_id).await,
    }
}

async fn heartbeat(cli: &Cli, node_id: String) -> Result<()> {
    let operations = match helpers::call(cli, RpcRequest::Handshake).await? {
        RpcResponse::CapabilityManifest(value) => {
            value.operations.into_iter().map(|value| value.id).collect()
        }
        other => return helpers::unexpected(other),
    };
    let node = NodeRecord {
        node_id,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        operations,
        last_seen_millis: util::now_millis(),
    };
    helpers::expect_accepted(
        cli,
        helpers::call(cli, RpcRequest::NodeHeartbeat { node }).await?,
    )
}
