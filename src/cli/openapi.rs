use super::Cli;
use clap::Args;
use hologram_live::error::{LiveError, Result};
use hologram_live::server::ApiDoc;
use std::path::PathBuf;
use utoipa::OpenApi;

#[derive(Debug, Clone, Args)]
pub struct OpenapiArgs {
    #[arg(long)]
    output: Option<PathBuf>,
}

pub async fn run(_cli: Cli, args: OpenapiArgs) -> Result<()> {
    let json = ApiDoc::openapi()
        .to_pretty_json()
        .map_err(|error| LiveError::Protocol(error.to_string()))?;
    if let Some(path) = args.output {
        tokio::fs::write(&path, json)
            .await
            .map_err(|error| LiveError::io(&path, error))?;
        println!("wrote {}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}
