//! Dev-only fixture plugin used by unit tests and the `s3_plugins` BDD suite.
//!
//! Serves the `hologram.live.plugin.v1` `PluginHost` contract on the Unix
//! domain socket named by `HOLOGRAM_PLUGIN_SOCKET` and declares a single
//! operation, `echo.ping`, which answers its JSON payload wrapped as
//! `{"echo": <payload>}`.

#[cfg(unix)]
mod unix_impl {
    use hologram_live::plugin::{pb, SOCKET_ENV};
    use hologram_live::protocol::PROTOCOL_VERSION;
    use std::path::PathBuf;
    use tonic::{Request, Response, Status};

    pub const PLUGIN_ID: &str = "dev.hologram.examples.echo";
    pub const OPERATION_ID: &str = "echo.ping";

    struct EchoPlugin;

    #[tonic::async_trait]
    impl pb::plugin_host_server::PluginHost for EchoPlugin {
        async fn describe(
            &self,
            _request: Request<pb::DescribeRequest>,
        ) -> Result<Response<pb::PluginDescriptor>, Status> {
            Ok(Response::new(pb::PluginDescriptor {
                id: PLUGIN_ID.to_owned(),
                name: "Echo example plugin".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                operations: vec![pb::PluginOperation {
                    id: OPERATION_ID.to_owned(),
                    kind: pb::PluginOperationKind::Mutation as i32,
                }],
                min_protocol: u32::from(PROTOCOL_VERSION),
            }))
        }

        async fn invoke(
            &self,
            request: Request<pb::InvokeRequest>,
        ) -> Result<Response<pb::InvokeResponse>, Status> {
            let request = request.into_inner();
            if request.operation != OPERATION_ID {
                return Err(Status::unimplemented(format!(
                    "unknown operation {}",
                    request.operation
                )));
            }
            let payload: serde_json::Value =
                serde_json::from_slice(&request.payload).map_err(|error| {
                    Status::invalid_argument(format!("payload must be JSON: {error}"))
                })?;
            let result = serde_json::json!({ "echo": payload });
            Ok(Response::new(pb::InvokeResponse {
                result: serde_json::to_vec(&result).expect("serialize echo result"),
                error_code: String::new(),
                error_message: String::new(),
            }))
        }

        async fn ping(
            &self,
            _request: Request<pb::PingRequest>,
        ) -> Result<Response<pb::PingResponse>, Status> {
            Ok(Response::new(pb::PingResponse {
                protocol_version: u32::from(PROTOCOL_VERSION),
            }))
        }
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let socket = PathBuf::from(
            std::env::var_os(SOCKET_ENV)
                .unwrap_or_else(|| panic!("{SOCKET_ENV} must name the plugin socket")),
        );
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket)?;
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
        tonic::transport::Server::builder()
            .add_service(pb::plugin_host_server::PluginHostServer::new(EchoPlugin))
            .serve_with_incoming(incoming)
            .await?;
        Ok(())
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    unix_impl::run().await
}

#[cfg(not(unix))]
fn main() {
    eprintln!("echo-plugin requires a unix host (unix domain socket transport)");
    std::process::exit(1);
}
