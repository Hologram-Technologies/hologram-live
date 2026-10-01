//! The iroh cluster transport (Phase 2a, #179).
//!
//! A second [`ClusterNetwork`], dialled by public key so a node with no
//! routable address still joins: `iroh:ed25519:<64 hex>` names a node id, and
//! the same 32 secret bytes behind `node.key` construct the iroh `SecretKey`,
//! so the `EndpointId` a peer dials **is** the `node_id` it already admitted.
//!
//! The endpoint speaks two protocols, dispatched on the connection's ALPN
//! before the handshake is awaited. `hologram/cluster/1` carries the cluster
//! router as HTTP/1 over a single bidirectional stream: accepted streams are
//! served by **the existing axum cluster router** (three routes, never the
//! application router — the request proof and admission layers apply
//! identically, since transport authentication is additional assurance, not a
//! substitute), and outbound requests go through hyper's client over the same
//! duplex, so there is one protocol implementation and no second wire format.
//! Phase 2b adds `iroh_blobs::ALPN`: object bytes move as BLAKE3/bao verified,
//! resumable streams through the node's shared blob store, behind a
//! connection-level admission gate, and never reach an HTTP route.

use super::blobs::{self, BlobChannel};
use super::identity::NodeIdentity;
use super::network::{ClusterNetwork, ClusterRequest, ClusterResponse};
use crate::app::AppState;
use crate::error::{LiveError, Result};
use axum::routing::{get, post};
use iroh::endpoint::VarInt;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, SecretKey};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::HashAndFormat;
use std::path::Path;
use std::sync::Arc;

/// The scheme prefix of a key-addressed cluster endpoint.
pub const SCHEME: &str = "iroh:";
/// The ALPN of the cluster router's HTTP-over-stream protocol.
pub const CLUSTER_ALPN: &[u8] = b"hologram/cluster/1";

/// The application error code a blobs connection is closed with when the
/// dialler is not in this node's admitted set. The fetch side recognizes it
/// and classifies the failure as authorization — the peer's decision about
/// this node, which ends a replication round — rather than as a fact about
/// the one object being fetched.
pub(crate) const BLOBS_REFUSED_NOT_ADMITTED: u32 = 0x1;

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
    /// The node's shared blob store: the dial side's fetches land here, and
    /// `cluster::run` hands a clone to the serve loop's provider side and to
    /// the mirror that keeps its tags a cache of the registry.
    blobs: FsStore,
}

impl IrohNetwork {
    /// Binds the endpoint from the node identity. `relays` and `discovery`
    /// come from configuration with the operator-cluster defaults — no
    /// relays, no publication — already validated. `blobs_dir` is where the
    /// node's blob store lives (`paths.data_dir/cluster-blobs`); it is
    /// opened here so the one store is shared by the provider side, the
    /// fetch side and the mirror from the start.
    pub async fn bind(
        identity: &NodeIdentity,
        relays: &[String],
        discovery: &str,
        blobs_dir: &Path,
    ) -> Result<Self> {
        let secret = SecretKey::from_bytes(&identity.secret_bytes());
        // Both ALPNs are registered on the one endpoint: the cluster router's
        // and the blobs protocol's. A connection offering anything else fails
        // its handshake, exactly as it did when only the cluster ALPN was
        // registered.
        let mut builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(secret)
            .alpns(vec![CLUSTER_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()]);
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
        let blobs = FsStore::load(blobs_dir).await.map_err(|error| {
            LiveError::Io(format!(
                "open cluster blob store {}: {error}",
                blobs_dir.display()
            ))
        })?;
        Ok(Self { endpoint, blobs })
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

    /// The node's shared blob store, cloned for the serve loop and the
    /// registry mirror. Cheap: the store is a channel pair to its actor.
    pub fn blobs(&self) -> &FsStore {
        &self.blobs
    }
}

/// Serves both protocols the endpoint speaks until it closes. The ALPN is
/// read before the handshake is awaited, so each connection is dispatched
/// before it is accepted: the cluster ALPN to the cluster router, the blobs
/// ALPN through the admission gate to the blob provider. A connection
/// offering anything else cannot get this far — only these two ALPNs are
/// registered — and its handshake fails, as it did before Phase 2b.
pub async fn serve(endpoint: Endpoint, state: AppState, blobs: FsStore) {
    let router = cluster_router(state.clone());
    loop {
        let Some(incoming) = endpoint.accept().await else {
            return;
        };
        let router = router.clone();
        let state = state.clone();
        let blobs = blobs.clone();
        tokio::spawn(async move {
            let mut accepting = match incoming.accept() {
                Ok(accepting) => accepting,
                Err(error) => {
                    tracing::debug!(%error, "iroh connection attempt failed");
                    return;
                }
            };
            let alpn = match accepting.alpn().await {
                Ok(alpn) => alpn,
                Err(error) => {
                    tracing::debug!(%error, "iroh handshake offered no usable ALPN");
                    return;
                }
            };
            let connection = match accepting.await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::debug!(%error, "iroh handshake failed");
                    return;
                }
            };
            if alpn == CLUSTER_ALPN {
                serve_cluster_connection(connection, router).await;
            } else if alpn.as_slice() == iroh_blobs::ALPN {
                serve_blobs_connection(connection, &state, blobs).await;
            }
        });
    }
}

/// The cluster router — three routes, never the application router — served
/// over one connection until the peer hangs up. Every request still carries
/// a request proof and passes admission: QUIC authenticating the transport
/// does not exempt a request from either.
async fn serve_cluster_connection(connection: iroh::endpoint::Connection, router: axum::Router) {
    loop {
        match connection.accept_bi().await {
            Ok((send, recv)) => {
                let router = router.clone();
                tokio::spawn(serve_stream(router, send, recv));
            }
            Err(_) => return,
        }
    }
}

/// A blobs-protocol connection. iroh-blobs has no request-level
/// authorization of its own — its security model is hash-as-capability — and
/// an object's hash is not secret: it appears in every inventory answer, in
/// OCI manifests, in logs. So the cluster's own gate is applied at the one
/// place it can be total: the dialler's key must be in this node's admitted
/// set — the same `Admission` the cluster router consults, read at accept
/// time — or the connection is closed before one request is read. One check
/// then covers every exchange on the connection, and there is no handler
/// surface behind it other than "serve bytes whose hash the dialler already
/// knows, from a set the registry already publishes to admitted peers".
///
/// No request proof rides here. The proof machinery exists to authenticate
/// *claims* — method, path, query, body — to a parser that will act on them;
/// a blobs request carries no claim: the hash is self-authenticating, and
/// bao verified streaming means the requester detects any byte that is not
/// the hash's content. The residual is availability, not integrity, and it
/// is bounded by the same admission decision that let the peer in.
async fn serve_blobs_connection(
    connection: iroh::endpoint::Connection,
    state: &AppState,
    blobs: FsStore,
) {
    if !blob_gate_allows(state.admission().as_ref(), &connection.remote_id()) {
        let node_id = format!(
            "ed25519:{}",
            crate::util::hex(connection.remote_id().as_bytes())
        );
        tracing::debug!(%node_id, "closing a blobs connection from an unadmitted peer");
        connection.close(
            VarInt::from_u32(BLOBS_REFUSED_NOT_ADMITTED),
            b"node is not admitted to this cluster",
        );
        return;
    }
    iroh_blobs::provider::handle_connection(
        connection,
        blobs.into(),
        iroh_blobs::provider::events::EventSender::default(),
    )
    .await;
}

/// The gate's whole decision: the dialler's endpoint key, spelled as the
/// `ed25519:<hex>` node id admission already keys on, is in the admitted set
/// read *now*. Read per connection, so a set that changed since the last
/// connection is honoured on this one.
fn blob_gate_allows(admission: &dyn super::admission::Admission, remote: &PublicKey) -> bool {
    let node_id = format!("ed25519:{}", crate::util::hex(remote.as_bytes()));
    admission.admitted().contains(&node_id)
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

/// The router the iroh listener serves, in one place so the tests that assert
/// its boundary (`an_iroh_peer_reaches_the_cluster_router_and_nothing_else`,
/// `the_blobs_alpn_reaches_no_http_route`) exercise exactly what production
/// serves.
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

    fn blob_channel(&self) -> Option<Arc<dyn BlobChannel>> {
        Some(Arc::new(IrohBlobChannel {
            endpoint: self.endpoint.clone(),
            blobs: self.blobs.clone(),
        }))
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

/// The blob channel over the same endpoint the cluster exchanges already
/// dial: iroh multiplexes both protocols over one QUIC connection to a peer,
/// so a replication round costs no extra handshakes for fetching what the
/// inventory named. All translation — peer address to public key, object id
/// to blob hash — happens here and nowhere else, which is what lets
/// `replicate_peer` speak in object ids.
struct IrohBlobChannel {
    endpoint: Endpoint,
    blobs: FsStore,
}

#[async_trait::async_trait]
impl BlobChannel for IrohBlobChannel {
    async fn fetch(&self, peer: &str, id: &str) -> Result<u64> {
        let hash = blobs::hash_from_object_id(id)?;
        let public_key = parse_address(peer)?;
        // A dial failure is about the peer — unreachable, timing out — so it
        // keeps the HTTP object route's classification: `Transport`, and the
        // round ends rather than retrying the same doomed dial per object.
        let connection = self
            .endpoint
            .connect(EndpointAddr::from(public_key), iroh_blobs::ALPN)
            .await
            .map_err(|error| LiveError::Transport(format!("reach cluster peer {peer}: {error}")))?;
        // `Remote::fetch` requests only the ranges the store does not already
        // hold, so an interrupted earlier attempt is resumed rather than
        // restarted; the stream verifies every range against the hash the id
        // names as it lands.
        let stats = self
            .blobs
            .remote()
            .fetch(connection, HashAndFormat::raw(hash))
            .complete()
            .await
            .map_err(|error| classify_fetch_error(peer, &error))?;
        Ok(stats.payload_bytes_read)
    }

    fn reader(&self, id: &str) -> Result<Box<dyn std::io::Read + Send>> {
        let hash = blobs::hash_from_object_id(id)?;
        let reader = self.blobs.blobs().reader(hash);
        // The registry's streaming put is synchronous; the bridge drives the
        // store's async reader on this runtime from inside the importer's
        // `spawn_blocking`, where blocking is allowed.
        Ok(Box::new(tokio_util::io::SyncIoBridge::new_with_handle(
            reader,
            tokio::runtime::Handle::current(),
        )))
    }
}

/// A fetch that fails after connecting is a fact about the one object, not
/// about the peer — its verified partial ranges persist and the next round
/// resumes them — so it is deliberately not the round-ending `Transport` the
/// dial above is. The one exception carries the provider's own verdict: a
/// close with `BLOBS_REFUSED_NOT_ADMITTED` means the peer has revoked this
/// node's access, which is what `ends_replication_round` already ends a
/// round on.
fn classify_fetch_error(peer: &str, error: &iroh_blobs::get::GetError) -> LiveError {
    if error.iroh_error_code() == Some(VarInt::from_u32(BLOBS_REFUSED_NOT_ADMITTED)) {
        return LiveError::Authorization(format!(
            "cluster peer {peer} refused blob access: this node is not admitted"
        ));
    }
    LiveError::Protocol(format!("fetch cluster object from {peer}: {error}"))
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
        let network = IrohNetwork::bind(&identity, &[], "none", &directory.path().join("blobs"))
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
    /// or DNS lookup provides in deployment. The identity is the caller's so
    /// a test can make the dialling key the same one its `AppState` signs
    /// with and its peer admits — in production both are the one `node.key`.
    async fn dialling_endpoint(
        identity: &NodeIdentity,
        blobs_dir: &std::path::Path,
        servers: &[Endpoint],
    ) -> (IrohNetwork, MemoryLookup) {
        let lookup = MemoryLookup::new();
        for server in servers {
            lookup.add_endpoint_info(server.addr());
        }
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&identity.secret_bytes()))
            .alpns(vec![CLUSTER_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()])
            .address_lookup(lookup.clone())
            .bind()
            .await
            .expect("bind the dialling endpoint");
        let blobs = FsStore::load(blobs_dir)
            .await
            .expect("open the dialler's blob store");
        (IrohNetwork { endpoint, blobs }, lookup)
    }

    async fn cluster_state(directory: &tempfile::TempDir) -> AppState {
        cluster_state_admitting(directory, &[]).await
    }

    /// A real `AppState` whose cluster admission trusts exactly `admitted`.
    /// With no advertised endpoint there is no cluster token, so admission is
    /// the configured allowlist — which is also what the blobs gate reads.
    async fn cluster_state_admitting(
        directory: &tempfile::TempDir,
        admitted: &[String],
    ) -> AppState {
        let mut config = crate::config::AppConfig::default();
        config.paths.config_dir = directory.path().join("config");
        config.paths.data_dir = directory.path().join("data");
        config.paths.state_dir = directory.path().join("state");
        config.paths.cache_dir = directory.path().join("cache");
        config.cluster.trusted_keys = admitted.to_vec();
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
        let server = IrohNetwork::bind(
            &server_identity,
            &[],
            "none",
            &server_dir.path().join("cluster-blobs"),
        )
        .await
        .expect("bind the serving endpoint");
        let server_address = server.local_node_address();
        let serve_endpoint = server.endpoint().clone();
        let serve_blobs = server.blobs().clone();
        tokio::spawn(async move {
            serve(serve_endpoint, state, serve_blobs).await;
        });

        let (client, _lookup) = dialling_endpoint(
            &identity_in(&client_dir),
            &client_dir.path().join("cluster-blobs"),
            &[server.endpoint().clone()],
        )
        .await;

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

    /// The admission gate's whole decision, exercised directly: an admitted
    /// key passes, an unknown one is refused, and because the set is read on
    /// every connection, a key admitted *between* two connections (here: by
    /// ticket, which pins it) passes on the second one.
    #[test]
    fn the_blobs_gate_admits_only_admitted_keys_and_reconsults_per_connection() {
        use super::super::admission::{ticket, Admission as _, Decision, TokenAdmission};

        let alice_dir = tempfile::tempdir().expect("alice's state directory");
        let alice = identity_in(&alice_dir);
        let alice_key =
            parse_address(&format!("{SCHEME}{}", alice.node_id())).expect("alice's address parses");
        let bob_dir = tempfile::tempdir().expect("bob's state directory");
        let bob = identity_in(&bob_dir);
        let bob_key =
            parse_address(&format!("{SCHEME}{}", bob.node_id())).expect("bob's address parses");

        let token = "a sufficiently long shared cluster admission token";
        let pins = tempfile::tempdir().expect("pin file directory");
        let admission = TokenAdmission::new(
            token.to_owned(),
            pins.path().join("cluster-pinned.json"),
            vec![alice.node_id()],
        )
        .expect("build token admission");

        assert!(blob_gate_allows(&admission, &alice_key));
        assert!(!blob_gate_allows(&admission, &bob_key));

        // Bob joins by ticket on the authenticated router and is pinned; the
        // *next* blobs connection from Bob must pass without the gate being
        // told anything changed.
        assert!(matches!(
            admission.authorize(&bob.node_id(), Some(&ticket(token, &bob.node_id()))),
            Decision::Admit
        ));
        assert!(blob_gate_allows(&admission, &bob_key));
    }

    /// One node, one object, one round: the pieces every blobs replication
    /// test below shares. `admit_client` decides whether the server's
    /// admission (and with it the blobs gate) trusts the dialler's key.
    struct BlobPair {
        server: IrohNetwork,
        server_address: String,
        client: IrohNetwork,
        client_state: AppState,
        stored_id: String,
        bytes: Vec<u8>,
        // Held only so the directories outlive the test: a dropped `TempDir`
        // deletes itself, and both nodes' stores live inside these.
        _server_dir: tempfile::TempDir,
        _client_dir: tempfile::TempDir,
    }

    async fn blob_pair(size: usize, admit_client: bool) -> BlobPair {
        crate::util::install_crypto_provider();
        let server_dir = tempfile::tempdir().expect("server state directory");
        let client_dir = tempfile::tempdir().expect("client state directory");

        // Both identities are created first, at the paths the two `AppState`s
        // will load, so each side's dialling key, proof signer and admitted
        // key are one — as the single `node.key` makes them in production.
        let server_state_dir = server_dir.path().join("state");
        std::fs::create_dir_all(&server_state_dir).expect("create the server state directory");
        let server_identity =
            NodeIdentity::load_or_create(&server_state_dir.join(super::super::identity::KEY_FILE))
                .expect("create the server identity");
        let client_state_dir = client_dir.path().join("state");
        std::fs::create_dir_all(&client_state_dir).expect("create the client state directory");
        let client_identity =
            NodeIdentity::load_or_create(&client_state_dir.join(super::super::identity::KEY_FILE))
                .expect("create the client identity");

        let server_state = if admit_client {
            cluster_state_admitting(&server_dir, &[client_identity.node_id()]).await
        } else {
            cluster_state(&server_dir).await
        };
        let mut block = Vec::with_capacity(64 * 1024);
        block.extend((0..=250_u8).cycle().take(64 * 1024));
        let mut bytes = Vec::with_capacity(size);
        while bytes.len() < size {
            bytes.extend_from_slice(&block);
        }
        bytes.truncate(size);
        let stored = server_state
            .registry()
            .put_object(
                "file".to_owned(),
                "text/plain".to_owned(),
                Some("object.bin".to_owned()),
                &bytes,
            )
            .expect("put the object into the server's registry");

        let server = IrohNetwork::bind(
            &server_identity,
            &[],
            "none",
            &server_dir.path().join("cluster-blobs"),
        )
        .await
        .expect("bind the serving endpoint");
        let server_address = server.local_node_address();
        // In deployment the mirror runs on the replication cadence from
        // `cluster::run`; the tests run the pass by hand before anyone dials.
        let mirrored = blobs::reconcile(&server_state, server.blobs())
            .await
            .expect("mirror the server's registry");
        assert_eq!(mirrored.imported, 1, "the server's object is mirrored");
        spawn_serve(&server, &server_state);

        let client_state = cluster_state(&client_dir).await;
        let (client, _lookup) = dialling_endpoint(
            &client_identity,
            &client_dir.path().join("cluster-blobs"),
            &[server.endpoint().clone()],
        )
        .await;

        BlobPair {
            server,
            server_address,
            client,
            client_state,
            stored_id: stored.id,
            bytes,
            _server_dir: server_dir,
            _client_dir: client_dir,
        }
    }

    /// Starts the serving half of a node in the background, exactly as
    /// `cluster::run` does: one task, one endpoint, both protocols.
    fn spawn_serve(network: &IrohNetwork, state: &AppState) {
        let endpoint = network.endpoint().clone();
        let blobs = network.blobs().clone();
        let state = state.clone();
        tokio::spawn(async move {
            serve(endpoint, state, blobs).await;
        });
    }

    /// The integration test Phase 2b exists for: the provider's registry
    /// holds an object the requester's does not; after one replication round
    /// the requester serves the identical bytes under the identical id, and
    /// its own blob store can serve the object onward (the mirror pass tags
    /// it). Nothing about the HTTP path changes — this pair speaks blobs end
    /// to end because both networks offer the channel.
    #[tokio::test]
    async fn a_missing_object_replicates_over_the_blobs_protocol() {
        let pair = blob_pair(300_000, true).await;
        let client_blobs = pair.client.blobs().clone();
        let networks = super::super::network::NetworkRegistry::new(vec![Arc::new(pair.client)]);

        super::super::replication::replicate_peer(
            &pair.client_state,
            &networks,
            &pair.server_address,
            "a token the allowlist never checks",
        )
        .await
        .expect("a replication round over the blobs protocol succeeds");

        let replicated = pair
            .client_state
            .registry()
            .get_object(&pair.stored_id)
            .expect("the requester's registry serves the object");
        assert_eq!(
            replicated.bytes, pair.bytes,
            "identical bytes under the identical id"
        );
        assert_eq!(replicated.metadata.media_type, "text/plain");
        assert_eq!(replicated.metadata.filename.as_deref(), Some("object.bin"));

        let mirrored = blobs::reconcile(&pair.client_state, &client_blobs)
            .await
            .expect("mirror the requester's registry");
        assert_eq!(mirrored.imported, 1);
        let digest = pair.stored_id.strip_prefix("blake3:").expect("blake3 id");
        assert!(
            client_blobs
                .tags()
                .get(blobs::tag_name(digest))
                .await
                .expect("read the tag")
                .is_some(),
            "the requester's blob store holds the named tag and can serve the object onward"
        );

        pair.server.endpoint().close().await;
    }

    /// The resume claim, scoped to what the substrate guarantees: a fetch
    /// abandoned mid-object leaves its verified partial ranges in the store,
    /// and the next fetch transfers only what is missing. Asserted by
    /// progress accounting — the second fetch moves fewer payload bytes than
    /// the object's size — not merely by eventual success. Loopback proves
    /// the mechanism; a crash of the store itself is not claimed.
    #[tokio::test]
    async fn an_interrupted_fetch_resumes_from_its_partial_ranges() {
        use iroh_blobs::api::remote::GetProgressItem;
        use tokio_stream::StreamExt;

        const SIZE: usize = 32 * 1024 * 1024;
        let pair = blob_pair(SIZE, true).await;
        let hash = blobs::hash_from_object_id(&pair.stored_id).expect("the id converts");
        let server_key = parse_address(&pair.server_address).expect("the address parses");
        let client_blobs = pair.client.blobs().clone();

        // First attempt: consume progress until some payload has landed, then
        // abandon the fetch by dropping it.
        let connection = pair
            .client
            .endpoint()
            .connect(EndpointAddr::from(server_key), iroh_blobs::ALPN)
            .await
            .expect("connect for the first attempt");
        let mut stream = Box::pin(
            client_blobs
                .remote()
                .fetch(connection, HashAndFormat::raw(hash))
                .stream(),
        );
        let mut landed = 0_u64;
        while let Some(item) = stream.next().await {
            match item {
                GetProgressItem::Progress(transferred) if transferred > 0 => {
                    landed = transferred;
                    break;
                }
                GetProgressItem::Done(_) => panic!(
                    "the whole {SIZE}-byte object arrived before the fetch could be abandoned; the test needs a bigger object"
                ),
                GetProgressItem::Error(error) => {
                    panic!("the first fetch failed before any progress: {error}")
                }
                GetProgressItem::Progress(_) => {}
            }
        }
        assert!(landed > 0, "the first fetch transferred payload bytes");
        drop(stream);
        let local = client_blobs
            .remote()
            .local(HashAndFormat::raw(hash))
            .await
            .expect("local info after the abandoned fetch");
        assert!(
            !local.is_complete(),
            "an abandoned fetch must leave a partial blob, not a complete one"
        );

        // Second attempt, a fresh connection: only the missing ranges move.
        let connection = pair
            .client
            .endpoint()
            .connect(EndpointAddr::from(server_key), iroh_blobs::ALPN)
            .await
            .expect("reconnect for the resumed attempt");
        let stats = client_blobs
            .remote()
            .fetch(connection, HashAndFormat::raw(hash))
            .complete()
            .await
            .expect("the resumed fetch completes");
        assert!(
            stats.payload_bytes_read > 0,
            "something remained to transfer"
        );
        assert!(
            stats.payload_bytes_read < SIZE as u64,
            "the resumed fetch moved {} bytes of a {SIZE}-byte object: it continued the partial ranges rather than restarting from byte zero",
            stats.payload_bytes_read
        );

        let mut reader = client_blobs.blobs().reader(hash);
        let mut read_back = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut read_back)
            .await
            .expect("read the completed blob back");
        assert_eq!(read_back, pair.bytes);

        pair.server.endpoint().close().await;
    }

    /// A dialler outside the admitted set knows the hash — it is not secret —
    /// and still gets nothing: the connection is closed before one request is
    /// read, so no ranges ever leave the provider.
    #[tokio::test]
    async fn an_unadmitted_dialler_gets_no_bytes_for_a_hash_it_knows() {
        let pair = blob_pair(64 * 1024, false).await;
        let hash = blobs::hash_from_object_id(&pair.stored_id).expect("the id converts");
        let server_key = parse_address(&pair.server_address).expect("the address parses");
        let client_blobs = pair.client.blobs().clone();

        let connection = pair
            .client
            .endpoint()
            .connect(EndpointAddr::from(server_key), iroh_blobs::ALPN)
            .await
            .expect("the handshake itself is not gated; admission is");
        let fetched = client_blobs
            .remote()
            .fetch(connection, HashAndFormat::raw(hash))
            .complete()
            .await;
        assert!(
            fetched.is_err(),
            "an unadmitted dialler's fetch must fail, got {fetched:?}"
        );
        let local = client_blobs
            .remote()
            .local(HashAndFormat::raw(hash))
            .await
            .expect("local info after the refused fetch");
        assert!(
            !local.is_complete(),
            "no complete blob may land from a refused connection"
        );

        pair.server.endpoint().close().await;
    }

    /// The one-handler-set rule, restated for the new protocol: a connection
    /// that negotiated the blobs ALPN reaches the blob provider and nothing
    /// else. An HTTP request written down it must not be answered as HTTP.
    #[tokio::test]
    async fn the_blobs_alpn_reaches_no_http_route() {
        use tokio::io::AsyncWriteExt;

        let pair = blob_pair(1024, true).await;
        let server_key = parse_address(&pair.server_address).expect("the address parses");

        let connection = pair
            .client
            .endpoint()
            .connect(EndpointAddr::from(server_key), iroh_blobs::ALPN)
            .await
            .expect("connect with the blobs ALPN");
        let (mut send, mut recv) = connection.open_bi().await.expect("open a stream");
        send.write_all(b"GET /api/v1/cluster/objects HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .expect("write an HTTP request down the blobs stream");
        send.shutdown().await.expect("finish the request");

        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            recv.read_to_end(usize::MAX),
        )
        .await;
        if let Ok(Ok(answered)) = read {
            assert!(
                !answered.starts_with(b"HTTP/"),
                "an HTTP response came back over the blobs ALPN: {answered:?}"
            );
        }
        // Any other outcome — a reset, an error, silence past the timeout —
        // is the protocol refusing the bytes, which is the point: there is no
        // HTTP listener behind this ALPN.

        pair.server.endpoint().close().await;
    }
}
