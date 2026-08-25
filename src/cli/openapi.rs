use super::{helpers, Cli};
use clap::Args;
use hologram_live::error::{LiveError, Result};
use hologram_live::module::ModuleRegistry;
use hologram_live::server::openapi_document;
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct OpenapiArgs {
    #[arg(long)]
    output: Option<PathBuf>,
}

pub async fn run(cli: Cli, args: OpenapiArgs) -> Result<()> {
    let (config, _) = super::helpers::load(&cli)?;
    let modules = ModuleRegistry::build(&config.modules.enabled)?;
    let json = openapi_document(&modules)
        .to_pretty_json()
        .map_err(|error| LiveError::Protocol(error.to_string()))?;
    if let Some(path) = args.output {
        tokio::fs::write(&path, json)
            .await
            .map_err(|error| LiveError::io(&path, error))?;
        if cli.json {
            helpers::print(
                &cli,
                &serde_json::json!({ "status": "written", "output": path }),
            )?;
        } else {
            println!("wrote {}", path.display());
        }
    } else {
        println!("{json}");
    }
    Ok(())
}
