use super::{helpers, Cli};
use clap::Args;
use hologram_live::client::LiveClient;
use hologram_live::error::{LiveError, Result};
use hologram_live::process;

#[derive(Debug, Clone, Args)]
pub struct RouteArgs {
    operation: String,
}

pub async fn run(cli: Cli, args: RouteArgs) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let request = helpers::request_for_operation(&args.operation)?;
    let client = LiveClient::from_config(&config)?;
    let decision = match client.explain_route(&request).await {
        Ok(value) => value,
        Err(LiveError::Transport(_) | LiveError::Capability(_)) => {
            process::ensure_daemon(&config, &path).await?;
            client.explain_route(&request).await?
        }
        Err(error) => return Err(error),
    };
    helpers::print(&cli, &decision)
}
