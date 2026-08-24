use super::{helpers, Cli};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

pub async fn run(cli: Cli) -> Result<()> {
    match helpers::call(&cli, RpcRequest::Health).await? {
        RpcResponse::Health(value) => helpers::print(&cli, &value),
        other => helpers::unexpected(other),
    }
}
