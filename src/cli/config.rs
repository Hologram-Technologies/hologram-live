use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;

#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ConfigCommand {
    Path,
    Validate,
    Show,
}

pub async fn run(cli: Cli, args: ConfigArgs) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    match args.command {
        ConfigCommand::Path => println!("{}", path.display()),
        ConfigCommand::Validate => println!("configuration is valid"),
        ConfigCommand::Show => println!("{}", toml::to_string_pretty(&config)?),
    }
    Ok(())
}
