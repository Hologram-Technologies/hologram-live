//! The iroh cluster transport (Phase 2a, #179).
//!
//! A second [`ClusterNetwork`], dialled by public key so a node with no
//! routable address still joins: `iroh:ed25519:<64 hex>` names a node id, and
//! the same 32 secret bytes behind `node.key` construct the iroh `SecretKey`,
//! so the `EndpointId` a peer dials **is** the `node_id` it already admitted.
//!
//! Both directions speak HTTP/1 over a single bidirectional stream: accepted
//! streams are served by **the existing axum cluster router** (three routes,
//! never the application router — the request proof and admission layers apply
//! identically, since transport authentication is additional assurance, not a
//! substitute), and outbound requests go through hyper's client over the same
//! duplex, so there is one protocol implementation and no second wire format.

use super::identity::NodeIdentity;
use super::network::{ClusterNetwork, ClusterRequest, ClusterResponse};
use crate::app::AppState;
use crate::error::{LiveError, Result};
use axum::routing::{get, post};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, SecretKey};

/// The scheme prefix of a key-addressed cluster endpoint.
pub const SCHEME: &str = "iroh:";
/// The one ALPN this transport serves and dials.
pub const CLUSTER_ALPN: &[u8] = b"hologram/cluster/1";

/// Parses `iroh:ed25519:<64 hex>` into the key being named. Configuration
/// validation has already accepted the address; this is the dial-time decode,
/// and it still refuses anything malformed rather than trusting the caller.
pub fn parse_address(address: &str) -> Result<PublicKey> {
    let node_id = address.strip_prefix(SCHEME).ok_or_else(|| {
        LiveError::Config(format!("cluster address is not key-addressed: {address}"))
    })?;
    let verifying = super::identity::parse_node_id(node_id)
        .map_err(|error| LiveError::Config(format!("invalid iroh cluster address: {error}")))?;
    PublicKey::from_bytes(verifying.as_bytes())
        .map_err(|error| LiveError::Config(format!("invalid iroh cluster address: {error}")))
}

pub struct IrohNetwork {
    endpoint: Endpoint,
}

impl IrohNetwork {
    /// Binds the endpoint from the node identity. `relays` and `discovery`
    /// come from configuration with the operator-cluster defaults — no
    /// relays, no publication — already validated.
    pub async fn bind(identity: &NodeIdentity, relays: &[String], discovery: &str) -> Result<Self> {
        let secret = SecretKey::from_bytes(&identity.secret_bytes());
        let mut builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(secret)
            .alpns(vec![CLUSTER_ALPN.to_vec()]);
        if relays.is_empty() {
            builder = builder.relay_mode(RelayMode::Disabled);
        } else {
            let urls = relays
                .iter()
                .map(|url| {
                    url.parse::<RelayUrl>().map_err(|error| {
                        LiveError::Config(format!("invalid cluster relay {url}: {error}"))
                    })
                })
                .collect::<Result<Vec<RelayUrl>>>()?;
            builder = builder.relay_mode(RelayMode::Custom(RelayMap::from_iter(urls)));
        }
        builder = match discovery {
            "none" => builder,
            "dns" => builder.address_lookup(iroh::address_lookup::dns::DnsAddressLookup::n0_dns()),
            // iroh 1.3 ships no mDNS address lookup, so the configuration key
            // names it but this build cannot honour it; the error is explicit
            // rather than a silent fallback to no discovery at all.
            "mdns" => {
                return Err(LiveError::Config(
                    "cluster.discovery = \"mdns\" is not available in this iroh build (iroh 1.3 ships no mDNS address lookup); use \"dns\" or \"none\""
                        .to_owned(),
                ));
            }
            other => {
                return Err(LiveError::Config(format!(
                    "cluster.discovery must be \"none\", \"mdns\" or \"dns\", not {other:?}"
                )));
            }
        };
        let endpoint = builder
            .bind()
            .await
            .map_err(|error| LiveError::Transport(format!("bind iroh endpoint: {error}")))?;
        Ok(Self { endpoint })
    }

    /// This node's key-addressed cluster address: the address is the identity.
    pub fn local_node_address(&self) -> String {
        format!(
            "{SCHEME}ed25519:{}",
            crate::util::hex(self.endpoint.id().as_bytes())
        )
    }

    /// The bound endpoint, cloned for the serve loop so the network itself can
    /// move into the registry.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

/// Serves the cluster router — three routes, never the application router —
/// until the endpoint closes. Every connection still carries a request proof
/// and passes admission: QUIC authenticating the transport does not exempt a
/// request from either.
pub async fn serve(endpoint: Endpoint, state: AppState) {
    let router = cluster_router(state);
    loop {
        let Some(incoming) = endpoint.accept().await else {
            return;
        };
        let router = router.clone();
        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::debug!(%error, "iroh handshake failed");
                    return;
                }
            };
            loop {
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        let router = router.clone();
                        tokio::spawn(serve_stream(router, send, recv));
                    }
                    Err(_) => return,
                }
            }
        });
    }
}

/// One accepted bidirectional stream is one duplex, served by hyper running
/// the cluster router. Nothing else is mounted: an application route dialled
/// over iroh finds no handler here.
async fn serve_stream(
    router: axum::Router,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
) {
    use tower::Service;

    let duplex = tokio::io::join(recv, send);
    let io = hyper_util::rt::TokioIo::new(duplex);
    let service = hyper::service::service_fn(move |request| {
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    if let Err(error) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .await
    {
        tracing::debug!(%error, "iroh cluster connection ended");
    }
}

/// The router the iroh listener serves, in one place so the test that asserts
/// its boundary (`iroh_listener_serves_only_the_cluster_router`) exercises
/// exactly what production serves.
fn cluster_router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route(
            super::JOIN_PATH,
            post(crate::modules::control_plane::join_cluster),
        )
        .route(
            super::OBJECTS_PATH,
            get(crate::modules::control_plane::list_cluster_objects),
        )
        .route(
            super::OBJECT_PATH,
            get(crate::modules::control_plane::get_cluster_object),
        )
        .with_state(state)
}

#[async_trait::async_trait]
impl ClusterNetwork for IrohNetwork {
    fn scheme(&self) -> &'static str {
        "iroh"
    }

    fn local_address(&self) -> Option<String> {
        Some(self.local_node_address())
    }

    fn accepts(&self, address: &str) -> bool {
        address.starts_with(SCHEME)
    }

    fn recipient_for(&self, address: &str) -> String {
        // The scheme is routing, not identity: the recipient names the node,
        // so an iroh proof carries the bare `ed25519:…` node id — the same
        // string `recipient_names_self` already accepts.
        address.strip_prefix(SCHEME).unwrap_or(address).to_owned()
    }

    async fn send(&self, peer: &str, request: ClusterRequest) -> Result<ClusterResponse> {
        let public_key = parse_address(peer)?;
        let connection = self
            .endpoint
            .connect(EndpointAddr::from(public_key), CLUSTER_ALPN)
            .await
            .map_err(|error| LiveError::Transport(format!("reach cluster peer {peer}: {error}")))?;
        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|error| LiveError::Transport(format!("open stream to {peer}: {error}")))?;
        let duplex = tokio::io::join(recv, send);
        let io = hyper_util::rt::TokioIo::new(duplex);
        let (mut sender, driver) =
            hyper::client::conn::http1::handshake(io)
                .await
                .map_err(|error| {
                    LiveError::Transport(format!("cluster handshake with {peer}: {error}"))
                })?;
        // The driver must be polled for the exchange to progress; it ends
        // when the sender drops.
        tokio::spawn(async move {
            let _ = driver.await;
        });

        let target = match &request.query {
            Some(query) => format!("{}?{query}", request.path),
            None => request.path.clone(),
        };
        let mut builder = hyper::Request::builder()
            .method(request.method)
            .uri(target)
            .header(super::proof::RECIPIENT_HEADER, &request.recipient)
            .header(super::proof::NODE_HEADER, &request.proof.node_id)
            .header(super::proof::TIMESTAMP_HEADER, &request.proof.timestamp)
            .header(super::proof::SIGNATURE_HEADER, &request.proof.signature);
        if let Some(ticket) = &request.ticket {
            builder = builder.header(super::proof::TICKET_HEADER, ticket);
        }
        if let Some(epoch) = &request.epoch {
            builder = builder.header(super::proof::EPOCH_HEADER, epoch);
        }
        if !request.body.is_empty() {
            builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
        }
        let outgoing = builder
            .body(http_body_util::Full::new(bytes::Bytes::from(request.body)))
            .map_err(|error| LiveError::Protocol(format!("build cluster request: {error}")))?;
        let response = sender.send_request(outgoing).await.map_err(|error| {
            LiveError::Transport(format!("cluster exchange with {peer}: {error}"))
        })?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        // The ceiling is enforced while the body is read, exactly as
        // `HttpNetwork::send` enforces it: a chunk that would cross the bound
        // fails the exchange before its bytes are held.
        let mut body = Vec::new();
        let mut incoming = response.into_body();
        while let Some(frame) = http_body_util::BodyExt::frame(&mut incoming).await {
            let frame = frame.map_err(|error| {
                LiveError::Transport(format!("read cluster peer {peer}: {error}"))
            })?;
            let Ok(data) = frame.into_data() else {
                continue;
            };
            if let Some(limit) = request.max_response_bytes {
                if (body.len() as u64).saturating_add(data.len() as u64) > limit {
                    return Err(LiveError::Capability(format!(
                        "cluster peer {peer} response exceeds its {limit} byte ceiling"
                    )));
                }
            }
            body.extend_from_slice(&data);
        }
        Ok(ClusterResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::address_lookup::MemoryLookup;
    use iroh::endpoint::presets;

    fn identity_in(directory: &tempfile::TempDir) -> NodeIdentity {
        NodeIdentity::load_or_create(&directory.path().join(super::super::identity::KEY_FILE))
            .expect("test identity")
    }

    #[test]
    fn an_iroh_address_names_the_key() {
        let directory = tempfile::tempdir().expect("state directory");
        let identity = identity_in(&directory);
        let address = format!("{SCHEME}{}", identity.node_id());

        let key = parse_address(&address).expect("a well-formed address parses");
        assert_eq!(
            crate::util::hex(key.as_bytes()),
            identity.node_id().trim_start_matches("ed25519:")
        );

        assert!(parse_address("iroh:ed25519:zz").is_err());
        assert!(parse_address("https://node.example").is_err());
    }

    #[tokio::test]
    async fn the_network_claims_only_keyed_addresses_and_names_the_node() {
        let directory = tempfile::tempdir().expect("state directory");
        let identity = identity_in(&directory);
        let network = IrohNetwork::bind(&identity, &[], "none")
            .await
            .expect("bind an endpoint");

        let address = format!("{SCHEME}{}", identity.node_id());
        assert!(network.accepts(&address));
        assert!(!network.accepts("https://node.example"));
        // The recipient is the bare node id: the scheme is routing, and the
        // receiver's `recipient_names_self` already accepts this form.
        assert_eq!(network.recipient_for(&address), identity.node_id());
        assert_eq!(network.local_address().as_deref(), Some(address.as_str()));
        network.endpoint().close().await;
    }

    /// Builds an endpoint that can dial `servers` over loopback without
    /// relays or discovery: the memory address lookup answers every key the
    /// test has inserted, which is exactly the resolution a configured relay
    /// or DNS lookup provides in deployment.
    async fn dialling_endpoint(
        directory: &tempfile::TempDir,
        servers: &[Endpoint],
    ) -> (IrohNetwork, MemoryLookup) {
        let identity = identity_in(directory);
        let lookup = MemoryLookup::new();
        for server in servers {
            lookup.add_endpoint_info(server.addr());
        }
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&identity.secret_bytes()))
            .alpns(vec![CLUSTER_ALPN.to_vec()])
            .address_lookup(lookup.clone())
            .bind()
            .await
            .expect("bind the dialling endpoint");
        (IrohNetwork { endpoint }, lookup)
    }

    async fn cluster_state(directory: &tempfile::TempDir) -> AppState {
        let mut config = crate::config::AppConfig::default();
        config.paths.config_dir = directory.path().join("config");
        config.paths.data_dir = directory.path().join("data");
        config.paths.state_dir = directory.path().join("state");
        config.paths.cache_dir = directory.path().join("cache");
        // `init_for_test` installs at most once per process; see
        // replication.rs's digest-mismatch test for why `init` would race.
        let tracing = crate::observability::init_for_test(&config.tracing, &config.telemetry)
            .expect("init the test tracing subscriber");
        AppState::build(config, tracing)
            .await
            .expect("build a real AppState backed by a temp dir")
    }

    /// The integration test Phase 2a exists for: two in-process endpoints over
    /// loopback, no relay and no discovery, the cluster router served over one
    /// and dialled from the other. Asserts the three things the design pins:
    /// a request reaches the cluster handler and is answered, the proof layer
    /// still refuses an unproven request, and the response ceiling is honoured
    /// while the body is read.
    #[tokio::test]
    async fn an_iroh_peer_reaches_the_cluster_router_and_nothing_else() {
        crate::util::install_crypto_provider();
        let server_dir = tempfile::tempdir().expect("server state directory");
        let client_dir = tempfile::tempdir().expect("client state directory");
        let state = cluster_state(&server_dir).await;

        let server_identity = identity_in(&server_dir);
        let server = IrohNetwork::bind(&server_identity, &[], "none")
            .await
            .expect("bind the serving endpoint");
        let server_address = server.local_node_address();
        let serve_endpoint = server.endpoint().clone();
        tokio::spawn(async move {
            serve(serve_endpoint, state).await;
        });

        let (client, _lookup) = dialling_endpoint(&client_dir, &[server.endpoint().clone()]).await;

        // 1. The cluster router answers. `/api/v1/cluster/objects` requires a
        // proof, so an unproven request must be refused by the proof layer —
        // which is also the proof the request reached the handler at all.
        let request = |path: &str, max_response_bytes| ClusterRequest {
            method: "GET",
            path: path.to_owned(),
            query: None,
            body: Vec::new(),
            recipient: server_identity.node_id(),
            proof: super::super::proof::RequestProof {
                node_id: "ed25519:00".to_owned(),
                timestamp: "0".to_owned(),
                signature: "00".to_owned(),
            },
            ticket: None,
            epoch: None,
            max_response_bytes,
        };
        let refused = client
            .send(&server_address, request(super::super::OBJECTS_PATH, None))
            .await
            .expect("the cluster route answers over iroh");
        assert_eq!(
            refused.status, 401,
            "an unproven request is refused by the proof layer, over iroh as over HTTP"
        );

        // 2. The listener serves only the cluster router: an application
        // route over iroh reaches no handler.
        let missing = client
            .send(&server_address, request("/healthz", None))
            .await
            .expect("a non-cluster route is still answered — by the 404");
        assert_eq!(missing.status, 404, "the application router is not mounted");

        // 3. The response ceiling is enforced while the body is read: even
        // the small 401 body crosses a one-byte ceiling, and the exchange
        // fails as a capability error rather than returning the body.
        let oversize = client
            .send(
                &server_address,
                request(super::super::OBJECTS_PATH, Some(1)),
            )
            .await;
        assert!(
            matches!(oversize, Err(LiveError::Capability(_))),
            "got {oversize:?}"
        );

        server.endpoint().close().await;
    }
}
