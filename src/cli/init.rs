use clap::Args;
use hologram_live::{config::AppConfig, error::Result};

use super::Cli;

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    #[arg(long)]
    force: bool,
}

pub async fn run(cli: Cli, init_args: InitArgs) -> Result<()> {
    let path = AppConfig::initialize(cli.config.as_deref(), init_args.force)?;
    println!("initialized {}", path.display());
    Ok(())
}
