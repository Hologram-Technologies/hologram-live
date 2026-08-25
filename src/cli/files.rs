use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::protocol::{ObjectContent, RpcRequest, RpcResponse};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct FilesArgs {
    #[command(subcommand)]
    command: FilesCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum FilesCommand {
    /// List stored file objects.
    List,
    /// Store a file and print its content-addressed metadata.
    Put {
        path: PathBuf,
        #[arg(long, default_value = "application/octet-stream")]
        media_type: String,
    },
    /// Download a stored file by object ID.
    Get {
        id: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Rename a stored file without changing its content-addressed ID.
    Rename {
        id: String,
        #[arg(allow_hyphen_values = true)]
        filename: String,
    },
}

pub async fn run(cli: Cli, args: FilesArgs) -> Result<()> {
    match args.command {
        FilesCommand::List => list(&cli).await,
        FilesCommand::Put { path, media_type } => put(&cli, path, media_type).await,
        FilesCommand::Get { id, output } => get(&cli, id, output).await,
        FilesCommand::Rename { id, filename } => rename(&cli, id, filename).await,
    }
}

async fn rename(cli: &Cli, id: String, filename: String) -> Result<()> {
    match helpers::call(cli, RpcRequest::FilesRename { id, filename }).await? {
        RpcResponse::Object(value) => helpers::print(cli, &value),
        other => helpers::unexpected(other),
    }
}

async fn list(cli: &Cli) -> Result<()> {
    match helpers::call(cli, RpcRequest::FilesList).await? {
        RpcResponse::Objects(value) => helpers::print(cli, &value),
        other => helpers::unexpected(other),
    }
}

async fn put(cli: &Cli, path: PathBuf, media_type: String) -> Result<()> {
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| LiveError::io(&path, error))?;
    let filename = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned());
    match helpers::call(
        cli,
        RpcRequest::FilesPut {
            media_type,
            filename,
            bytes,
        },
    )
    .await?
    {
        RpcResponse::Object(value) => helpers::print(cli, &value),
        other => helpers::unexpected(other),
    }
}

async fn get(cli: &Cli, id: String, output: Option<PathBuf>) -> Result<()> {
    match helpers::call(cli, RpcRequest::FilesGet { id }).await? {
        RpcResponse::ObjectContent(object) => write_download(cli, object, output).await,
        other => helpers::unexpected(other),
    }
}

#[derive(Debug, Serialize)]
struct DownloadReport {
    id: String,
    output: PathBuf,
    byte_length: u64,
}

pub(crate) async fn write_download(
    cli: &Cli,
    object: ObjectContent,
    output: Option<PathBuf>,
) -> Result<()> {
    let output = output.unwrap_or_else(|| default_output(&object));
    let byte_length = object.bytes.len().try_into().unwrap_or(u64::MAX);
    tokio::fs::write(&output, object.bytes)
        .await
        .map_err(|error| LiveError::io(&output, error))?;
    if cli.json {
        helpers::print(
            cli,
            &DownloadReport {
                id: object.metadata.id,
                output,
                byte_length,
            },
        )
    } else {
        helpers::message(cli, "written", format!("wrote {}", output.display()))
    }
}

fn default_output(object: &ObjectContent) -> PathBuf {
    object
        .metadata
        .filename
        .as_deref()
        .and_then(|filename| Path::new(filename).file_name())
        .map_or_else(
            || PathBuf::from(object.metadata.id.replace(':', "_")),
            PathBuf::from,
        )
}
