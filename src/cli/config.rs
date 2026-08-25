use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use serde_json::json;

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
        ConfigCommand::Path if cli.json => helpers::print(&cli, &json!({ "path": path })),
        ConfigCommand::Path => helpers::message(&cli, "ok", path.display().to_string()),
        ConfigCommand::Validate => helpers::message(&cli, "valid", "configuration is valid"),
        ConfigCommand::Show if cli.json => helpers::print(&cli, &config),
        ConfigCommand::Show => {
            println!("{}", toml::to_string_pretty(&config)?);
            Ok(())
        }
    }
}
