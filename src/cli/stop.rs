use super::{helpers, Cli};
use hologram_live::client::LiveClient;
use hologram_live::error::{LiveError, Result};
use hologram_live::protocol::{RpcRequest, RpcResponse};

pub async fn run(cli: Cli) -> Result<()> {
    let (config, _) = helpers::load(&cli)?;
    let client = LiveClient::from_config(&config)?;
    match client.call(RpcRequest::Shutdown).await {
        Ok(RpcResponse::Accepted) => {
            helpers::message(&cli, "stopping", "hologram daemon stopping")?;
        }
        Ok(other) => return helpers::unexpected(other),
        Err(LiveError::Transport(_)) => {
            helpers::message(&cli, "not_running", "hologram daemon is not running")?;
        }
        Err(error) => return Err(error),
    }
    Ok(())
}
