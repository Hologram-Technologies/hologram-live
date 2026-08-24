use super::Cli;
use hologram_live::client::LiveClient;
use hologram_live::config::AppConfig;
use hologram_live::error::{LiveError, Result};
use hologram_live::process;
use hologram_live::protocol::{self, RpcRequest, RpcResponse};
use serde::Serialize;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

pub fn load(cli: &Cli) -> Result<(AppConfig, PathBuf)> {
    let (config, path) = AppConfig::load(cli.config.as_deref())?;
    config.create_directories()?;
    Ok((config, path))
}

pub async fn call(cli: &Cli, request: RpcRequest) -> Result<RpcResponse> {
    let (config, path) = load(cli)?;
    call_with_local_start(&config, &path, request).await
}

pub async fn call_with_local_start(
    config: &AppConfig,
    path: &Path,
    request: RpcRequest,
) -> Result<RpcResponse> {
    let client = LiveClient::from_config(config)?;
    match client.call(request.clone()).await {
        Ok(response) => Ok(response),
        Err(LiveError::Transport(_) | LiveError::Capability(_)) => {
            process::ensure_daemon(config, path).await?;
            client.call(request).await
        }
        Err(error) => Err(error),
    }
}

pub fn print<T: Serialize + Debug>(cli: &Cli, value: &T) -> Result<()> {
    if cli.json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{value:#?}");
    }
    Ok(())
}

pub fn expect_accepted(response: RpcResponse) -> Result<()> {
    match response {
        RpcResponse::Accepted => Ok(()),
        other => unexpected(other),
    }
}

pub fn unexpected<T>(response: RpcResponse) -> Result<T> {
    Err(LiveError::Protocol(format!(
        "unexpected response: {response:?}"
    )))
}

pub fn request_for_operation(value: &str) -> Result<RpcRequest> {
    use protocol::operation;
    match value {
        operation::SYSTEM_HANDSHAKE => Ok(RpcRequest::Handshake),
        operation::SYSTEM_HEALTH => Ok(RpcRequest::Health),
        operation::MODULES_LIST => Ok(RpcRequest::ModulesList),
        operation::TRACING_GET => Ok(RpcRequest::TracingGet),
        operation::REGISTRY_LIST => Ok(RpcRequest::RegistryList),
        operation::FILES_LIST => Ok(RpcRequest::FilesList),
        operation::HOLO_LIST => Ok(RpcRequest::HoloList),
        operation::HOLO_RESIDENT => Ok(RpcRequest::HoloResident),
        operation::HISTORY_LIST => Ok(RpcRequest::HistoryList),
        operation::NODES_LIST => Ok(RpcRequest::NodesList),
        _ => Err(LiveError::Protocol(format!(
            "route explanation requires a parameter-free known operation; got {value}"
        ))),
    }
}
