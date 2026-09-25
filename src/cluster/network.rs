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
        let body = response
            .bytes()
            .await
            .map_err(|error| LiveError::Transport(format!("read cluster peer {peer}: {error}")))?
            .to_vec();
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
