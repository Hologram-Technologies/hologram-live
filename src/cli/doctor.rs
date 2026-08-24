use super::{helpers, Cli};
use hologram_live::client::LiveClient;
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let modules = hologram_live::module::ModuleRegistry::build(&config.modules.enabled)?;
    println!("configuration: {}", path.display());
    println!("modules:       {} resolved", modules.info().len());
    println!("config root:   {}", config.paths.config_dir.display());
    println!("data root:     {}", config.paths.data_dir.display());
    println!("state root:    {}", config.paths.state_dir.display());
    match LiveClient::from_config(&config)?
        .call(RpcRequest::Health)
        .await
    {
        Ok(RpcResponse::Health(value)) => println!("server:        {}", value.status),
        _ => println!("server:        stopped"),
    }
    Ok(())
}
