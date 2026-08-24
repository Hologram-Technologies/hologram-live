use crate::config::{AppConfig, TargetPreference};
use crate::error::{LiveError, Result};
use crate::grpc::pb;
use crate::protocol::{CapabilityManifest, OperationKind, RpcRequest, RpcResponse};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::Mutex;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig};

#[derive(Debug, Clone)]
struct Endpoint {
    name: String,
    url: String,
    token: Option<String>,
    channel: Channel,
}

pub struct LiveClient {
    endpoints: Vec<Endpoint>,
    allow_read_fallback: bool,
    manifests: Mutex<BTreeMap<String, CapabilityManifest>>,
    maximum_message_bytes: usize,
}

impl LiveClient {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let token = config.auth_token();
        let timeout = Duration::from_secs(config.client.request_timeout_secs);
        let local = endpoint(
            "local",
            &config.client.local_endpoint,
            token.clone(),
            timeout,
        )?;
        let remote = config
            .client
            .remote_endpoint
            .as_ref()
            .map(|url| endpoint("remote", url, token, timeout))
            .transpose()?;
        let mut endpoints = Vec::new();
        match config.client.preference {
            TargetPreference::Local => {
                endpoints.push(local);
                if let Some(remote) = remote {
                    endpoints.push(remote);
                }
            }
            TargetPreference::Remote => {
                if let Some(remote) = remote {
                    endpoints.push(remote);
                }
                endpoints.push(local);
            }
        }
        Ok(Self {
            endpoints,
            allow_read_fallback: config.client.allow_read_fallback,
            manifests: Mutex::new(BTreeMap::new()),
            maximum_message_bytes: config.server.max_rpc_bytes,
        })
    }

    pub async fn call(&self, request: RpcRequest) -> Result<RpcResponse> {
        let operation = request.operation();
        let kind = request.kind();
        let mut capability_misses = Vec::new();
        let mut transport_errors = Vec::new();

        for endpoint in &self.endpoints {
            if !matches!(request, RpcRequest::Handshake) {
                match self.handshake(endpoint).await {
                    Ok(manifest) => {
                        if !manifest.operations.iter().any(|item| item.id == operation) {
                            capability_misses.push(endpoint.name.clone());
                            continue;
                        }
                    }
                    Err(error @ (LiveError::Authentication(_) | LiveError::Authorization(_))) => {
                        return Err(error)
                    }
                    Err(error) => {
                        if kind != OperationKind::Read || !self.allow_read_fallback {
                            return Err(error);
                        }
                        transport_errors.push(format!("{} handshake: {error}", endpoint.name));
                        continue;
                    }
                }
            }
            match self.send_direct(endpoint, &request).await {
                Ok(RpcResponse::Error(error)) => return Err(error.into()),
                Ok(response) => return Ok(response),
                Err(error @ (LiveError::Authentication(_) | LiveError::Authorization(_))) => {
                    return Err(error)
                }
                Err(error) => {
                    if kind != OperationKind::Read {
                        return Err(LiveError::UnknownCommitState(format!(
                            "{} may have received mutation {operation}: {error}",
                            endpoint.name
                        )));
                    }
                    if !self.allow_read_fallback || is_security_error(&error) {
                        return Err(error);
                    }
                    transport_errors.push(format!("{}: {error}", endpoint.name));
                }
            }
        }

        if !capability_misses.is_empty() {
            return Err(LiveError::Capability(format!(
                "no configured target provides {operation}; missing on {}",
                capability_misses.join(", ")
            )));
        }
        Err(LiveError::Transport(format!(
            "all configured targets failed: {}",
            transport_errors.join("; ")
        )))
    }

    pub async fn handshake_local(&self) -> Result<CapabilityManifest> {
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.name == "local")
            .ok_or_else(|| LiveError::Config("no local endpoint configured".to_owned()))?;
        self.handshake(endpoint).await
    }

    async fn handshake(&self, endpoint: &Endpoint) -> Result<CapabilityManifest> {
        if let Some(manifest) = self.manifests.lock().await.get(&endpoint.url).cloned() {
            return Ok(manifest);
        }
        let response = self.send_direct(endpoint, &RpcRequest::Handshake).await?;
        let manifest = match response {
            RpcResponse::CapabilityManifest(manifest) => manifest,
            RpcResponse::Error(error) => return Err(error.into()),
            other => {
                return Err(LiveError::Protocol(format!(
                    "{} returned {other:?} to handshake",
                    endpoint.name
                )))
            }
        };
        self.manifests
            .lock()
            .await
            .insert(endpoint.url.clone(), manifest.clone());
        Ok(manifest)
    }

    async fn send_direct(&self, endpoint: &Endpoint, request: &RpcRequest) -> Result<RpcResponse> {
        let mut client =
            pb::hologram_live_client::HologramLiveClient::new(endpoint.channel.clone())
                .max_decoding_message_size(self.maximum_message_bytes)
                .max_encoding_message_size(self.maximum_message_bytes);
        let mut grpc_request = tonic::Request::new(pb::RpcRequest::from(request.clone()));
        if let Some(token) = &endpoint.token {
            let value = MetadataValue::try_from(format!("Bearer {token}"))
                .map_err(|error| LiveError::Authentication(format!("invalid token: {error}")))?;
            grpc_request.metadata_mut().insert("authorization", value);
        }
        let response = client
            .call(grpc_request)
            .await
            .map_err(map_grpc_status)?
            .into_inner();
        RpcResponse::try_from(response)
    }
}

fn endpoint(name: &str, url: &str, token: Option<String>, timeout: Duration) -> Result<Endpoint> {
    let url = trim_slash(url);
    let mut transport = tonic::transport::Endpoint::from_shared(url.clone())
        .map_err(|error| LiveError::Config(format!("invalid {name} endpoint: {error}")))?
        .timeout(timeout)
        .connect_timeout(timeout)
        .user_agent(format!("hologram/{}", env!("CARGO_PKG_VERSION")))
        .map_err(|error| LiveError::Config(format!("invalid user agent: {error}")))?;
    if url.starts_with("https://") {
        transport = transport
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .map_err(|error| LiveError::Config(format!("configure {name} TLS: {error}")))?;
    }
    Ok(Endpoint {
        name: name.to_owned(),
        url,
        token,
        channel: transport.connect_lazy(),
    })
}

fn map_grpc_status(status: tonic::Status) -> LiveError {
    let message = status.message().to_owned();
    match status.code() {
        tonic::Code::Unauthenticated => return LiveError::Authentication(message),
        tonic::Code::PermissionDenied => return LiveError::Authorization(message),
        tonic::Code::Unimplemented => return LiveError::Capability(message),
        tonic::Code::NotFound => return LiveError::NotFound(message),
        tonic::Code::FailedPrecondition | tonic::Code::AlreadyExists => {
            return LiveError::Conflict(message)
        }
        tonic::Code::InvalidArgument | tonic::Code::OutOfRange => {
            return LiveError::Protocol(message)
        }
        _ => {}
    }
    let message = status.to_string();
    let lowercase = message.to_ascii_lowercase();
    if lowercase.contains("certificate")
        || lowercase.contains("tls")
        || lowercase.contains("unknown issuer")
    {
        LiveError::Authentication(format!("secure transport validation failed: {message}"))
    } else {
        LiveError::Transport(message)
    }
}

fn is_security_error(error: &LiveError) -> bool {
    matches!(
        error,
        LiveError::Authentication(_) | LiveError::Authorization(_)
    )
}

fn trim_slash(value: &str) -> String {
    value.trim_end_matches('/').to_owned()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteDecision {
    pub operation: String,
    pub target: String,
    pub endpoint: String,
    pub reason: String,
}

impl LiveClient {
    /// Explain which configured authority would receive an operation without
    /// dispatching the operation itself. Capability negotiation is performed
    /// against each target in configured preference order.
    pub async fn explain_route(&self, request: &RpcRequest) -> Result<RouteDecision> {
        let operation = request.operation();
        let mut failures = Vec::new();
        for endpoint in &self.endpoints {
            match self.handshake(endpoint).await {
                Ok(manifest) => {
                    if manifest.operations.iter().any(|item| item.id == operation) {
                        return Ok(RouteDecision {
                            operation: operation.to_owned(),
                            target: endpoint.name.clone(),
                            endpoint: endpoint.url.clone(),
                            reason: format!(
                                "{} advertises operation {operation} under native protocol v{}",
                                endpoint.name, manifest.protocol_version
                            ),
                        });
                    }
                    failures.push(format!("{} does not advertise {operation}", endpoint.name));
                }
                Err(error @ (LiveError::Authentication(_) | LiveError::Authorization(_))) => {
                    return Err(error)
                }
                Err(error) => failures.push(format!("{}: {error}", endpoint.name)),
            }
        }
        Err(LiveError::Capability(format!(
            "no configured target can serve {operation}: {}",
            failures.join("; ")
        )))
    }
}
