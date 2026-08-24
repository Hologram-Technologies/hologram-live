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
            println!("installed hologram {}", update::install(&config).await?);
            Ok(())
        }
        UpdateCommand::Rollback => {
            update::rollback()?;
            println!("rolled back hologram");
            Ok(())
        }
    }
}
