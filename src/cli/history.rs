use super::{helpers, Cli};
use clap::{Args, Subcommand, ValueEnum};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct HistoryArgs {
    #[command(subcommand)]
    command: HistoryCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum HistoryCommand {
    New {
        title: String,
    },
    List,
    Show {
        id: String,
    },
    Append {
        id: String,
        #[arg(long, value_enum, default_value = "user")]
        role: MessageRole,
        content: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

pub async fn run(cli: Cli, args: HistoryArgs) -> Result<()> {
    let request = match args.command {
        HistoryCommand::New { title } => RpcRequest::HistoryCreate { title },
        HistoryCommand::List => RpcRequest::HistoryList,
        HistoryCommand::Show { id } => RpcRequest::HistoryGet { id },
        HistoryCommand::Append { id, role, content } => RpcRequest::HistoryAppend {
            id,
            role: role.as_str().to_owned(),
            content,
        },
        HistoryCommand::Delete { id } => RpcRequest::HistoryDelete { id },
    };
    match helpers::call(&cli, request).await? {
        RpcResponse::Conversation(value) => helpers::print(&cli, &value),
        RpcResponse::Conversations(value) => helpers::print(&cli, &value),
        RpcResponse::Accepted => Ok(()),
        other => helpers::unexpected(other),
    }
}
