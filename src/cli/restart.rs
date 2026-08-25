use super::{helpers, Cli};
use hologram_live::client::LiveClient;
use hologram_live::error::Result;
use hologram_live::process;
use hologram_live::protocol::RpcRequest;

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let client = LiveClient::from_config(&config)?;
    let previous_pid = process::read_pid(&config).ok();
    let _ = client.call(RpcRequest::Shutdown).await;
    process::wait_stopped(&config, previous_pid).await?;
    let pid = process::start_daemon(&config, &path).await?;
    if cli.json {
        helpers::print(
            &cli,
            &serde_json::json!({ "status": "restarted", "pid": pid }),
        )
    } else {
        helpers::message(&cli, "restarted", "hologram daemon restarted")
    }
}
