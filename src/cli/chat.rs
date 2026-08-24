use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct ChatArgs {
    #[command(subcommand)]
    command: ChatCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ChatCommand {
    /// Send a message to a conversation and record the echoed response.
    Send { id: String, content: String },
}

pub async fn run(cli: Cli, args: ChatArgs) -> Result<()> {
    let request = match args.command {
        ChatCommand::Send { id, content } => RpcRequest::ChatSend { id, content },
    };
    match helpers::call(&cli, request).await? {
        RpcResponse::Conversation(value) => helpers::print(&cli, &value),
        other => helpers::unexpected(other),
    }
}
