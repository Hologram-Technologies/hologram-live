//! TLS on the registry's listener (`http.tls.certificate`, `.key`,
//! `.minimumtls`), as the reference serves it (plan P6 T2, FR-008, FR-S05).
//!
//! An accept loop, not axum's `Listener`: each connection's handshake runs on
//! its own task with a 10 s limit, so one client that never finishes it cannot
//! hold up the next. ALPN offers `h2` and `http/1.1`. Connections are drained
//! on the shutdown signal as the plain listener drains them. A request is
//! marked [`ServedOverTls`], so `Location` is written with `https`, as Go's
//! URL builder does when `r.TLS` is set.

use crate::error::{LiveError, Result};
use axum::Router;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// How long a client may take to finish the handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long a client may take to send a request's headers.
const HEADERS: Duration = Duration::from_secs(30);

/// What Go's `net/http` answers a plain HTTP request on a TLS port, byte for
/// byte (`server.go`, `serve`), when the first five bytes are one of
/// [`LOOKS_LIKE_HTTP`] (`tlsRecordHeaderLooksLikeHTTP`).
const PLAIN_ON_TLS: &[u8] =
    b"HTTP/1.0 400 Bad Request\r\n\r\nClient sent an HTTP request to an HTTPS server.\n";
const LOOKS_LIKE_HTTP: [&[u8; 5]; 5] = [b"GET /", b"HEAD ", b"POST ", b"PUT /", b"OPTIO"];

/// The oldest protocol version the listener accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimumTls {
    Tls12,
    Tls13,
}

impl MinimumTls {
    /// `http.tls.minimumtls` as the reference writes it; its default is 1.2.
    ///
    /// # Errors
    ///
    /// `tls1.0` and `tls1.1`, which this listener does not offer, and anything else.
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value.trim() {
            "" | "tls1.2" => Ok(Self::Tls12),
            "tls1.3" => Ok(Self::Tls13),
            // The reference knows only these two as well ("unknown minimum TLS level").
            "tls1.0" | "tls1.1" => Err(format!(
                "{value} is not offered: this registry speaks TLS 1.2 and 1.3 only"
            )),
            other => Err(format!(
                "{other:?} is not a TLS version: use tls1.2 or tls1.3"
            )),
        }
    }
}

/// The listener's certificate, key and minimum version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsSettings {
    /// PEM: the certificate, then any intermediates, all sent.
    pub certificate: PathBuf,
    /// PEM: PKCS#8, PKCS#1 or SEC1.
    pub key: PathBuf,
    pub minimum: MinimumTls,
}

/// Marks a request that arrived over TLS.
#[derive(Debug, Clone, Copy)]
pub struct ServedOverTls;

/// Load the certificate and key and build the acceptor, before the listener
/// binds: a certificate that cannot be read stops the start.
///
/// # Errors
///
/// `LiveError::Config` naming the file and what is wrong with it.
pub fn acceptor(settings: &TlsSettings) -> Result<TlsAcceptor> {
    crate::util::install_crypto_provider();
    let bad = |path: &Path, what: &str| {
        LiveError::Config(format!("http.tls: {}: {what}", path.display()))
    };
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&settings.certificate)
        .map_err(|error| bad(&settings.certificate, &error.to_string()))?
        .collect::<std::result::Result<_, _>>()
        .map_err(|error| bad(&settings.certificate, &error.to_string()))?;
    if chain.is_empty() {
        return Err(bad(&settings.certificate, "holds no certificate"));
    }
    let key = PrivateKeyDer::from_pem_file(&settings.key)
        .map_err(|error| bad(&settings.key, &error.to_string()))?;
    let versions: &[&rustls::SupportedProtocolVersion] = match settings.minimum {
        MinimumTls::Tls12 => &[&rustls::version::TLS13, &rustls::version::TLS12],
        MinimumTls::Tls13 => &[&rustls::version::TLS13],
    };
    let mut config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| bad(&settings.key, &error.to_string()))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Serve `router` over TLS until `shutdown`, then drain open connections for
/// at most `drain` (`server.graceful_shutdown_secs`).
///
/// # Errors
///
/// None today; the signature matches the plain listener's.
pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    router: Router,
    shutdown: impl Future<Output = ()> + Send,
    drain: Duration,
) -> Result<()> {
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(accepted) => accepted,
                    // As axum's loop: a client that went away is nothing; anything
                    // else (a full file table) is waited out, not fatal.
                    Err(error) if is_connection_error(&error) => continue,
                    Err(error) => {
                        tracing::warn!(%error, "accept failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                let watcher = graceful.watcher();
                let (acceptor, router) = (acceptor.clone(), router.clone());
                tokio::spawn(async move {
                    if let Err(reason) = connection(stream, acceptor, router, watcher).await {
                        tracing::debug!(%peer, reason = reason.as_str(), "TLS connection ended early");
                    }
                });
            }
            () = &mut shutdown => break,
        }
    }
    drop(listener);
    if tokio::time::timeout(drain, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!(
            seconds = drain.as_secs(),
            "connections still open after the drain; closing them"
        );
    }
    Ok(())
}

fn is_connection_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

async fn connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    router: Router,
    watcher: hyper_util::server::graceful::Watcher,
) -> std::result::Result<(), String> {
    let work = async {
        // A client speaking plain HTTP gets Go's answer, not a TLS alert.
        // A TLS record starts with 0x14 to 0x17, SSLv2 with 0x80 or above.
        let mut first = [0_u8; 5];
        let mut seen = 0;
        while seen < first.len() {
            let n = stream
                .peek(&mut first)
                .await
                .map_err(|error| error.to_string())?;
            if n == seen || !first[0].is_ascii_uppercase() {
                break;
            }
            seen = n;
        }
        if LOOKS_LIKE_HTTP.contains(&&first) {
            return Ok(Err(stream));
        }
        acceptor
            .accept(stream)
            .await
            .map(Ok)
            .map_err(|error| error.to_string())
    };
    let tls = match tokio::time::timeout(HANDSHAKE, work).await {
        Err(_) => return Err("the handshake took longer than 10 s".to_owned()),
        Ok(Err(reason)) => return Err(reason),
        Ok(Ok(Err(mut plain))) => {
            let _ = plain.write_all(PLAIN_ON_TLS).await;
            let _ = plain.shutdown().await;
            // Read what the client sent before closing: closing with unread
            // bytes sends a reset, and on Windows a reset throws away the
            // answer the client has not read yet.
            let mut sink = [0_u8; 4096];
            let drain = async {
                while matches!(tokio::io::AsyncReadExt::read(&mut plain, &mut sink).await, Ok(n) if n > 0)
                {
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
            return Err("a plain HTTP request on the TLS port".to_owned());
        }
        Ok(Ok(Ok(tls))) => tls,
    };
    let service = router.map_request(|mut request: axum::http::Request<_>| {
        request.extensions_mut().insert(ServedOverTls);
        request
    });
    let service = hyper_util::service::TowerToHyperService::new(service);
    let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    // Once the handshake is done, the headers still have a limit: a client
    // that sends a byte and waits holds nothing for long.
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HEADERS);
    let served = builder.serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(tls), service);
    watcher
        .watch(served.into_owned())
        .await
        .map_err(|error| error.to_string())
}
