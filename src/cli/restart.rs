use super::{helpers, Cli};
use hologram_live::client::LiveClient;
use hologram_live::error::Result;
use hologram_live::process;
use hologram_live::protocol::RpcRequest;

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let client = LiveClient::from_config(&config)?;
    let _ = client.call(RpcRequest::Shutdown).await;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&config.server.listen)
            .await
            .is_err()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    process::start_daemon(&config, &path).await?;
    println!("hologram daemon restarted");
    Ok(())
}
