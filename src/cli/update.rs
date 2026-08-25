use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::update;

#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    #[command(subcommand)]
    command: UpdateCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum UpdateCommand {
    Check,
    Apply,
    Rollback,
}

pub async fn run(cli: Cli, args: UpdateArgs) -> Result<()> {
    let (config, _) = helpers::load(&cli)?;
    match args.command {
        UpdateCommand::Check => helpers::print(&cli, &update::check(&config).await?),
        UpdateCommand::Apply => {
            let version = update::install(&config).await?;
            if cli.json {
                helpers::print(
                    &cli,
                    &serde_json::json!({ "status": "installed", "version": version }),
                )
            } else {
                helpers::message(&cli, "installed", format!("installed hologram {version}"))
            }
        }
        UpdateCommand::Rollback => {
            update::rollback()?;
            helpers::message(&cli, "rolled_back", "rolled back hologram")
        }
    }
}
