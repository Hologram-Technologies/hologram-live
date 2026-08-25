use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct PluginsArgs {
    #[command(subcommand)]
    command: PluginsCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum PluginsCommand {
    /// List allowlisted plugin modules and their runtime status.
    List,
    /// Invoke a plugin operation with a JSON payload.
    Call {
        /// Allowlisted plugin id (see `hologram plugins list`).
        plugin_id: String,
        /// Operation id the plugin declared in its Describe handshake.
        operation: String,
        /// JSON payload forwarded verbatim to the plugin.
        payload: String,
    },
}

pub async fn run(cli: Cli, args: PluginsArgs) -> Result<()> {
    match args.command {
        PluginsCommand::List => match helpers::call(&cli, RpcRequest::PluginList).await? {
            RpcResponse::Plugins(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        PluginsCommand::Call {
            plugin_id,
            operation,
            payload,
        } => match helpers::call(
            &cli,
            RpcRequest::PluginCall {
                plugin_id,
                operation,
                payload,
            },
        )
        .await?
        {
            // The result is already a JSON document; print it verbatim so
            // both `--json` consumers and humans see the same payload.
            RpcResponse::PluginResult(json) => {
                println!("{json}");
                Ok(())
            }
            other => helpers::unexpected(other),
        },
    }
}
