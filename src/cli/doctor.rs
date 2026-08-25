use super::{helpers, Cli};
use hologram_live::client::LiveClient;
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
struct DoctorReport {
    configuration: PathBuf,
    modules_resolved: usize,
    config_root: PathBuf,
    data_root: PathBuf,
    state_root: PathBuf,
    server: String,
}

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let modules = hologram_live::module::ModuleRegistry::build(&config.modules.enabled)?;
    let server = match LiveClient::from_config(&config)?
        .call(RpcRequest::Health)
        .await
    {
        Ok(RpcResponse::Health(value)) => value.status,
        _ => "stopped".to_owned(),
    };
    let report = DoctorReport {
        configuration: path,
        modules_resolved: modules.info().len(),
        config_root: config.paths.config_dir,
        data_root: config.paths.data_dir,
        state_root: config.paths.state_dir,
        server,
    };
    if cli.json {
        helpers::print(&cli, &report)
    } else {
        println!("configuration: {}", report.configuration.display());
        println!("modules:       {} resolved", report.modules_resolved);
        println!("config root:   {}", report.config_root.display());
        println!("data root:     {}", report.data_root.display());
        println!("state root:    {}", report.state_root.display());
        println!("server:        {}", report.server);
        Ok(())
    }
}
