//! TLS on the registry's public listener, from the reference's settings.
//!
//! `http.tls.certificate`, `http.tls.key` and `http.tls.minimumtls` (P6 T2).
//! The handshake runs in its own task per connection with a timeout, so a
//! client that opens sockets and never speaks holds nothing but its own
//! task, never the listener; ALPN offers HTTP/2 and HTTP/1.1, and the same
//! router answers both.

use crate::config::TlsConfig;
use crate::error::{LiveError, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

/// How long a connection may spend handshaking before it is dropped.
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// The ALPN names offered, HTTP/2 first.
const ALPN: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// Build the acceptor from the operator's certificate and key, before the
/// listener binds: a registry that cannot serve TLS must fail its start,
/// not its first handshake.
///
/// # Errors
///
/// `LiveError::Config` naming the file, when a certificate or key is
/// missing, unreadable, or not a readable PEM pair.
pub fn build(config: &TlsConfig) -> Result<TlsAcceptor> {
    let certificates: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(&config.certificate)
            .map_err(|error| {
                LiveError::Config(format!(
                    "registry TLS certificate {}: {error}",
                    config.certificate.display()
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| {
                LiveError::Config(format!(
                    "registry TLS certificate {}: {error}",
                    config.certificate.display()
                ))
            })?;
    if certificates.is_empty() {
        return Err(LiveError::Config(format!(
            "registry TLS certificate {} holds no certificate",
            config.certificate.display()
        )));
    }
    let key = PrivateKeyDer::from_pem_file(&config.key).map_err(|error| {
        LiveError::Config(format!(
            "registry TLS key {}: {error}",
            config.key.display()
        ))
    })?;
    let versions: &[&rustls::SupportedProtocolVersion] = match config.minimum {
        crate::config::TlsVersion::Tls12 => &[&rustls::version::TLS12, &rustls::version::TLS13],
        crate::config::TlsVersion::Tls13 => &[&rustls::version::TLS13],
    };
    // The provider is chosen here rather than process-wide: the registry must
    // serve ring whether or not anything else has installed a default.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .map_err(|error| {
            LiveError::Config(format!(
                "registry TLS: no protocol versions enabled (minimum {}): {error}",
                config.minimum.as_str()
            ))
        })?
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map_err(|error| {
            LiveError::Config(format!(
                "registry TLS: the certificate and the key do not match: {error}"
            ))
        })?;
    server.alpn_protocols = ALPN.iter().map(|name| name.to_vec()).collect();
    Ok(TlsAcceptor::from(Arc::new(server)))
}

/// Serve `router` over TLS until `shutdown`, draining open connections the
/// same way the plain listener does. Every accepted socket handshakes in its
/// own task; the listener never waits on one.
///
/// # Errors
///
/// `LiveError::Transport` when serving itself fails.
pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    router: axum::Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let address = listener
        .local_addr()
        .map_err(|error| LiveError::Transport(format!("TLS listener address: {error}")))?;
    // Handshake tasks hand established connections over here; the accept
    // loop owns the socket, so aborting it on shutdown closes the listener
    // while handshakes in flight expire on their own timeout.
    let (sender, receiver) = mpsc::unbounded_channel();
    let pump = tokio::spawn(accept_loop(listener, acceptor, sender));
    let served = axum::serve(TlsListener { address, receiver }, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            pump.abort();
        })
        .await
        .map_err(|error| LiveError::Transport(format!("serve TLS: {error}")));
    served
}

/// One established TLS connection and its peer.
type Established = (
    tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    std::net::SocketAddr,
);

async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    sender: mpsc::UnboundedSender<Established>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let acceptor = acceptor.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    let handshake = tokio::time::timeout(
                        Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
                        acceptor.accept(stream),
                    );
                    match handshake.await {
                        Ok(Ok(tls)) => {
                            let _ = sender.send((tls, peer));
                        }
                        Ok(Err(error)) => {
                            tracing::debug!(%peer, %error, "TLS handshake failed");
                        }
                        Err(_) => tracing::debug!(%peer, "TLS handshake timed out"),
                    }
                });
            }
            // The reference registry logs and keeps accepting; a transient
            // accept error must not end the server.
            Err(error) => {
                tracing::warn!(%error, "TLS accept failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// The established-connection end of the listener, as `axum::serve` reads it.
struct TlsListener {
    address: std::net::SocketAddr,
    receiver: mpsc::UnboundedReceiver<Established>,
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.receiver.recv().await {
                Some(established) => return established,
                // The accept loop is gone (shutdown aborted it); hold here,
                // the graceful shutdown ends the serve.
                None => std::future::pending::<()>().await,
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.address)
    }
}
