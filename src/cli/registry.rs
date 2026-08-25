use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    command: RegistryCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum RegistryCommand {
    List,
    Put {
        path: PathBuf,
        #[arg(long, default_value = "object")]
        kind: String,
        #[arg(long, default_value = "application/octet-stream")]
        media_type: String,
    },
    Get {
        id: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

pub async fn run(cli: Cli, args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommand::List => match helpers::call(&cli, RpcRequest::RegistryList).await? {
            RpcResponse::Objects(value) => helpers::print(&cli, &value),
            other => helpers::unexpected(other),
        },
        RegistryCommand::Put {
            path,
            kind,
            media_type,
        } => {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|error| hologram_live::error::LiveError::io(&path, error))?;
            let filename = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned());
            match helpers::call(
                &cli,
                RpcRequest::RegistryPut {
                    kind,
                    media_type,
                    filename,
                    bytes,
                },
            )
            .await?
            {
                RpcResponse::Object(value) => helpers::print(&cli, &value),
                other => helpers::unexpected(other),
            }
        }
        RegistryCommand::Get { id, output } => {
            match helpers::call(&cli, RpcRequest::RegistryGet { id }).await? {
                RpcResponse::ObjectContent(object) => {
                    super::files::write_download(&cli, object, output).await
                }
                other => helpers::unexpected(other),
            }
        }
    }
}
