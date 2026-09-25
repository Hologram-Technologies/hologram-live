//! The cluster's transport, as a trait.
//!
//! No network is privileged. HTTP is the only implementation in Phase 1; iroh
//! arrives in Phase 2, and Veilid or Reticulum could follow, as peers of it. A
//! cluster may run several at once, because an address carries its own scheme
//! and the registry routes on it — which is how a cluster migrates one node at
//! a time.
//!
//! Identity deliberately does not live here. The request proof authenticates the
//! request rather than the connection, so it travels unchanged over any network,
//! and a network that authenticates its own connections adds assurance without
//! being required for safety.

use crate::cluster::proof::RequestProof;
use crate::error::{LiveError, Result};
use std::sync::Arc;

/// `Debug` is implemented by hand below rather than derived: this struct carries
/// an admission ticket and a signature, and a derived one would print both the
/// moment anyone logged a request.
#[derive(Clone)]
pub struct ClusterRequest {
    pub method: &'static str,
    pub path: String,
    pub query: Option<String>,
    pub body: Vec<u8>,
    /// The recipient the caller signed the proof against — always the
    /// normalized origin it is dialling (`cluster::recipient_for`), never a
    /// peer-reported identity. Carried as data because it is already bound
    /// into `proof`'s signature: a network that recomputed it could silently
    /// address the request elsewhere, so no network is given the chance.
    pub recipient: String,
    pub proof: RequestProof,
    pub ticket: Option<String>,
    pub epoch: Option<String>,
    /// The most response body this request will accept, enforced *while* the
    /// answer is read. Part of the request rather than of a network's
    /// configuration so that every implementation is held to the caller's
    /// bound and none can widen it: `None` means the caller has no bound to
    /// impose, not that a peer may send anything it likes to a caller that
    /// does.
    pub max_response_bytes: Option<u64>,
}

impl std::fmt::Debug for ClusterRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClusterRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("query", &self.query)
            .field("body_bytes", &self.body.len())
            .field("recipient", &self.recipient)
            .field("node_id", &self.proof.node_id)
            .field("timestamp", &self.proof.timestamp)
            .field("ticket", &self.ticket.as_ref().map(|_| "<redacted>"))
            .field("signature", &"<redacted>")
            .field("epoch", &self.epoch)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ClusterResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ClusterResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub const fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

#[async_trait::async_trait]
pub trait ClusterNetwork: Send + Sync {
    /// The address prefix this network claims, e.g. `"https"` or `"iroh"`.
    ///
    /// Routing goes through [`ClusterNetwork::accepts`], so this is the
    /// network's name for diagnostics and for the registry's own tests rather
    /// than something the send path consults.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "names a network for diagnostics; routing asks `accepts` instead"
        )
    )]
    fn scheme(&self) -> &'static str;
    /// This node's address on this network, as peers should record it.
    fn local_address(&self) -> Option<String>;
    /// Whether this network can reach the given address.
    fn accepts(&self, address: &str) -> bool;
    /// One request/response exchange with a peer.
    ///
    /// An implementation **must** stop reading as soon as the accumulated body
    /// exceeds `request.max_response_bytes` and fail with
    /// [`LiveError::Capability`], rather than reading to completion and
    /// rejecting what it has already put in memory. A `ClusterResponse` hands
    /// back a whole body, so this is the only place that bound can be a bound
    /// on memory and not merely a verdict: an admitted-but-hostile peer will
    /// otherwise answer a small request with an arbitrarily large body.
    /// `Capability` because a caller iterating many objects treats an oversize
    /// one as a fact about that object, not about the peer.
    async fn send(&self, peer: &str, request: ClusterRequest) -> Result<ClusterResponse>;
    /// Addresses learned without configuration. An empty list is a valid answer.
    ///
    /// Nothing calls this yet: HTTP learns peers from the signed join response
    /// and the node directory, so it has nothing of its own to discover. Phase
    /// 2's iroh network answers it from its discovery services.
    #[expect(
        dead_code,
        reason = "the seam Phase 2's iroh discovery fills; HTTP has nothing to discover"
    )]
    async fn discover(&self, _limit: usize) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

pub struct HttpNetwork {
    client: reqwest::Client,
    advertised: Option<String>,
}

impl HttpNetwork {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            advertised: None,
        }
    }

    pub fn with_advertised(mut self, advertised: Option<String>) -> Self {
        self.advertised = advertised;
        self
    }
}

#[async_trait::async_trait]
impl ClusterNetwork for HttpNetwork {
    fn scheme(&self) -> &'static str {
        "https"
    }

    fn local_address(&self) -> Option<String> {
        self.advertised.clone()
    }

    fn accepts(&self, address: &str) -> bool {
        address.starts_with("https://") || address.starts_with("http://")
    }

    async fn send(&self, peer: &str, request: ClusterRequest) -> Result<ClusterResponse> {
        let mut url = reqwest::Url::parse(peer)
            .map_err(|error| LiveError::Config(format!("invalid cluster address: {error}")))?;
        url.set_path(&request.path);
        url.set_query(request.query.as_deref());

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| LiveError::Protocol(format!("invalid cluster method: {error}")))?;
        let mut builder = self
            .client
            .request(method, url)
            .header(crate::cluster::proof::RECIPIENT_HEADER, &request.recipient)
            .header(crate::cluster::proof::NODE_HEADER, &request.proof.node_id)
            .header(
                crate::cluster::proof::TIMESTAMP_HEADER,
                &request.proof.timestamp,
            )
            .header(
                crate::cluster::proof::SIGNATURE_HEADER,
                &request.proof.signature,
            );
        if let Some(ticket) = &request.ticket {
            builder = builder.header(crate::cluster::proof::TICKET_HEADER, ticket);
        }
        if let Some(epoch) = &request.epoch {
            builder = builder.header(crate::cluster::proof::EPOCH_HEADER, epoch);
        }
        if !request.body.is_empty() {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request.body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| LiveError::Transport(format!("reach cluster peer {peer}: {error}")))?;
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
        // Read a chunk at a time and stop the moment the ceiling is crossed.
        // `bytes()` would buffer whatever the peer chose to send before anyone
        // could object, which is exactly the memory the ceiling exists to deny
        // it. The check precedes the append, so nothing over the ceiling is
        // ever held.
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| LiveError::Transport(format!("read cluster peer {peer}: {error}")))?
        {
            if let Some(limit) = request.max_response_bytes {
                if (body.len() as u64).saturating_add(chunk.len() as u64) > limit {
                    return Err(LiveError::Capability(format!(
                        "cluster peer {peer} response exceeds its {limit} byte ceiling"
                    )));
                }
            }
            body.extend_from_slice(&chunk);
        }
        Ok(ClusterResponse {
            status,
            headers,
            body,
        })
    }
}

pub struct NetworkRegistry {
    networks: Vec<Arc<dyn ClusterNetwork>>,
}

impl NetworkRegistry {
    pub fn new(networks: Vec<Arc<dyn ClusterNetwork>>) -> Self {
        Self { networks }
    }

    pub fn route(&self, address: &str) -> Option<&Arc<dyn ClusterNetwork>> {
        self.networks
            .iter()
            .find(|network| network.accepts(address))
    }

    pub async fn send(&self, address: &str, request: ClusterRequest) -> Result<ClusterResponse> {
        let network = self.route(address).ok_or_else(|| {
            LiveError::Transport(format!("no cluster network can reach {address}"))
        })?;
        network.send(address, request).await
    }

    /// Every address this node can be reached at, across all networks.
    pub fn local_addresses(&self) -> Vec<String> {
        self.networks
            .iter()
            .filter_map(|network| network.local_address())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubNetwork {
        scheme: &'static str,
    }

    #[async_trait::async_trait]
    impl ClusterNetwork for StubNetwork {
        fn scheme(&self) -> &'static str {
            self.scheme
        }
        fn local_address(&self) -> Option<String> {
            Some(format!("{}:self", self.scheme))
        }
        fn accepts(&self, address: &str) -> bool {
            address.starts_with(self.scheme)
        }
        async fn send(&self, peer: &str, _request: ClusterRequest) -> Result<ClusterResponse> {
            Ok(ClusterResponse {
                status: 200,
                headers: vec![("x-peer".to_owned(), peer.to_owned())],
                body: self.scheme.as_bytes().to_vec(),
            })
        }
        async fn discover(&self, _limit: usize) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    fn request() -> ClusterRequest {
        ClusterRequest {
            method: "GET",
            path: "/api/v1/cluster/objects".to_owned(),
            query: None,
            body: Vec::new(),
            recipient: "https://node.example".to_owned(),
            proof: crate::cluster::proof::RequestProof {
                node_id: "ed25519:aa".to_owned(),
                timestamp: "0".to_owned(),
                signature: "00".to_owned(),
            },
            ticket: None,
            epoch: None,
            max_response_bytes: None,
        }
    }

    #[test]
    fn the_registry_routes_an_address_to_the_network_that_claims_it() {
        let registry = NetworkRegistry::new(vec![
            Arc::new(StubNetwork { scheme: "https" }),
            Arc::new(StubNetwork { scheme: "iroh" }),
        ]);
        assert_eq!(
            registry.route("https://node.example").map(|n| n.scheme()),
            Some("https")
        );
        assert_eq!(
            registry.route("iroh:ed25519:aa").map(|n| n.scheme()),
            Some("iroh")
        );
        assert!(registry.route("veilid:xyz").is_none());
    }

    // A mixed cluster is the point: migration happens one node at a time.
    #[tokio::test]
    async fn a_mixed_cluster_reaches_both_kinds_of_peer() {
        let registry = NetworkRegistry::new(vec![
            Arc::new(StubNetwork { scheme: "https" }),
            Arc::new(StubNetwork { scheme: "iroh" }),
        ]);
        let over_http = registry
            .send("https://node.example", request())
            .await
            .expect("http peer");
        let over_iroh = registry
            .send("iroh:ed25519:aa", request())
            .await
            .expect("iroh peer");
        assert_eq!(over_http.body, b"https".to_vec());
        assert_eq!(over_iroh.body, b"iroh".to_vec());
    }

    #[tokio::test]
    async fn an_unroutable_address_is_a_transport_error_not_a_panic() {
        let registry = NetworkRegistry::new(vec![Arc::new(StubNetwork { scheme: "https" })]);
        let error = registry
            .send("veilid:xyz", request())
            .await
            .expect_err("no network claims this address");
        assert!(matches!(error, LiveError::Transport(_)), "got {error:?}");
    }

    // Fix round 1: the ceiling has to stop an oversize body *during* the read,
    // not notice one after it is already in memory. This peer promises ten
    // gigabytes, writes at most `BUDGET` of them, and then goes quiet without
    // hanging up. An implementation that read to completion would still be
    // waiting for the rest when the `timeout` fires, and would have buffered
    // the whole budget on the way — so both assertions below fail for it,
    // while an implementation that abandons the read at the ceiling returns in
    // milliseconds having accepted almost nothing.
    #[tokio::test]
    async fn an_oversize_body_is_refused_before_it_is_buffered() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const PROMISED: u64 = 10 * 1024 * 1024 * 1024;
        const BUDGET: u64 = 64 * 1024 * 1024;
        const CEILING: u64 = 4096;
        /// Comfortably above anything the kernel's loopback buffers can absorb
        /// after the client stops reading, and far below `BUDGET`.
        const ACCEPTABLE: u64 = 16 * 1024 * 1024;

        crate::util::install_crypto_provider();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port for the oversize peer");
        let address = listener.local_addr().expect("read the bound address");
        let written = Arc::new(AtomicU64::new(0));
        let peer_written = Arc::clone(&written);
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut discarded = [0_u8; 4096];
            let _ = stream.read(&mut discarded).await;
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {PROMISED}\r\n\r\n");
            if stream.write_all(header.as_bytes()).await.is_err() {
                return;
            }
            let chunk = vec![b'x'; 64 * 1024];
            while peer_written.load(Ordering::Relaxed) < BUDGET {
                if stream.write_all(&chunk).await.is_err() {
                    return;
                }
                peer_written.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
            // Quiet, but still connected: a client waiting for the rest of the
            // promised body waits for the test's timeout rather than for a
            // convenient end-of-stream.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });

        let network = HttpNetwork::new(reqwest::Client::new());
        let mut oversize = request();
        oversize.max_response_bytes = Some(CEILING);
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            network.send(&format!("http://{address}"), oversize),
        )
        .await
        .expect("the ceiling must abort the read rather than wait out the body")
        .expect_err("a body over the ceiling is an error");
        // `Capability`, because `replication::ends_replication_round` treats
        // that as a fact about this one object and keeps the round going —
        // which is what an oversize object has always done.
        assert!(matches!(error, LiveError::Capability(_)), "got {error:?}");
        let written = written.load(Ordering::Relaxed);
        assert!(
            written < ACCEPTABLE,
            "the peer placed {written} bytes of its promised {PROMISED}: the read was not abandoned at the ceiling"
        );
    }

    #[test]
    fn the_http_network_claims_only_http_addresses() {
        // `reqwest::Client::new()` panics without a rustls provider under this
        // tree's `rustls-no-provider` feature, the same reason
        // `replication.rs`'s tests install one first.
        crate::util::install_crypto_provider();
        let network = HttpNetwork::new(reqwest::Client::new());
        assert!(network.accepts("https://node.example:11435"));
        assert!(network.accepts("http://127.0.0.1:11435"));
        assert!(!network.accepts("iroh:ed25519:aa"));
    }
}
