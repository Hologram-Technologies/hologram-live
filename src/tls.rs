//! TLS on the registry's listener (`http.tls.certificate`, `.key`,
//! `.minimumtls`), as the reference serves it (plan P6 T2, FR-008, FR-S05).
//!
//! An accept loop, not axum's `Listener`: each connection's handshake runs on
//! its own task with a 10 s limit, so one client that never finishes it cannot
//! hold up the next. ALPN offers `h2` and `http/1.1`. Connections are drained
//! on the shutdown signal as the plain listener drains them. A request is
//! marked [`ServedOverTls`], so `Location` is written with `https`, as Go's
//! URL builder does when `r.TLS` is set.
//!
//! The certificate and key are read again when they change (FR-S05,
//! `operations.md`): the files are checked every [`WATCH`], and new
//! handshakes use the new pair once it loads. A pair that does not load
//! leaves the old one serving, counts a failure, and logs one error line;
//! it is tried again when either file changes. Connections already open keep
//! the certificate they started with.

use crate::error::{LiveError, Result};
use axum::Router;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// How long a client may take to finish the handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long a client may take to send a request's headers.
const HEADERS: Duration = Duration::from_secs(30);
/// How often the certificate and key files are checked for a change: a
/// renewal is served within 5 s (`operations.md`, failure table).
const WATCH: Duration = Duration::from_secs(2);

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

/// The listener's TLS: the acceptor, and the certificate it presents, which
/// the watcher replaces in place.
pub struct Tls {
    acceptor: TlsAcceptor,
    current: Arc<Current>,
    settings: TlsSettings,
}

/// The pair new handshakes are given.
#[derive(Debug)]
struct Current(RwLock<Arc<CertifiedKey>>);

impl ResolvesServerCert for Current {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(
            self.0
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }
}

/// `hologram_tls_*` on `/metrics`: set once TLS is on.
static TLS_ON: AtomicBool = AtomicBool::new(false);
static NOT_AFTER: AtomicI64 = AtomicI64::new(0);
static RELOAD_FAILURES: AtomicU64 = AtomicU64::new(0);

/// The TLS lines of `/metrics`, empty when the listener is plain.
#[must_use]
pub fn metrics_text() -> String {
    if !TLS_ON.load(Ordering::Relaxed) {
        return String::new();
    }
    format!(
        "# HELP hologram_tls_certificate_not_after_seconds When the certificate being served expires, in Unix seconds.\n\
         # TYPE hologram_tls_certificate_not_after_seconds gauge\n\
         hologram_tls_certificate_not_after_seconds {}\n\
         # HELP hologram_tls_reload_failures_total Changed certificate or key files that did not load; the old pair kept serving.\n\
         # TYPE hologram_tls_reload_failures_total counter\n\
         hologram_tls_reload_failures_total {}\n",
        NOT_AFTER.load(Ordering::Relaxed),
        RELOAD_FAILURES.load(Ordering::Relaxed)
    )
}

/// Read the pair, and check that the key is the certificate's.
fn load(settings: &TlsSettings) -> Result<Arc<CertifiedKey>> {
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
    let certified = CertifiedKey::from_der(chain, key, &rustls::crypto::ring::default_provider())
        .map_err(|error| bad(&settings.key, &error.to_string()))?;
    Ok(Arc::new(certified))
}

/// Load the certificate and key and build the acceptor, before the listener
/// binds: a certificate that cannot be read stops the start.
///
/// # Errors
///
/// `LiveError::Config` naming the file and what is wrong with it.
pub fn acceptor(settings: &TlsSettings) -> Result<Tls> {
    crate::util::install_crypto_provider();
    let certified = load(settings)?;
    publish(&certified);
    let current = Arc::new(Current(RwLock::new(certified)));
    let versions: &[&rustls::SupportedProtocolVersion] = match settings.minimum {
        MinimumTls::Tls12 => &[&rustls::version::TLS13, &rustls::version::TLS12],
        MinimumTls::Tls13 => &[&rustls::version::TLS13],
    };
    let mut config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_cert_resolver(current.clone());
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    TLS_ON.store(true, Ordering::Relaxed);
    Ok(Tls {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        current,
        settings: settings.clone(),
    })
}

/// The gauge follows the certificate being served.
fn publish(certified: &CertifiedKey) {
    let expires = certified
        .end_entity_cert()
        .ok()
        .and_then(|cert| not_after(cert.as_ref()))
        .unwrap_or(0);
    NOT_AFTER.store(expires, Ordering::Relaxed);
}

/// Both files' bytes, or why they could not be read: what "changed" means.
/// The content, not the modification time, so a Kubernetes secret swapped by
/// symlink and a copy that keeps the old time are both seen.
fn fingerprint(settings: &TlsSettings) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for path in [&settings.certificate, &settings.key] {
        match std::fs::read(path) {
            Ok(bytes) => {
                hasher.update(&(bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(error) => {
                hasher.update(error.to_string().as_bytes());
            }
        }
    }
    hasher.finalize()
}

/// Watch the files, and swap the pair when they change and load.
async fn watch(current: Arc<Current>, settings: TlsSettings) {
    let read = |settings: TlsSettings| async move {
        tokio::task::spawn_blocking(move || fingerprint(&settings))
            .await
            .ok()
    };
    let Some(mut seen) = read(settings.clone()).await else {
        return;
    };
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + WATCH, WATCH);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let Some(now) = read(settings.clone()).await else {
            return;
        };
        if now == seen {
            continue;
        }
        // One attempt per change: a pair that fails is tried again only when
        // a file changes again, so a half-written renewal logs once.
        seen = now;
        let loaded = {
            let settings = settings.clone();
            tokio::task::spawn_blocking(move || load(&settings)).await
        };
        match loaded {
            Ok(Ok(certified)) => {
                publish(&certified);
                *current.0.write().unwrap_or_else(PoisonError::into_inner) = certified;
                tracing::info!(
                    certificate = %settings.certificate.display(),
                    not_after = NOT_AFTER.load(Ordering::Relaxed),
                    "TLS certificate reloaded"
                );
            }
            Ok(Err(error)) => {
                RELOAD_FAILURES.fetch_add(1, Ordering::Relaxed);
                tracing::error!(%error, "TLS certificate not reloaded; the old one is still served");
            }
            Err(_) => return,
        }
    }
}

/// A certificate's `notAfter`, in Unix seconds, read from its DER. Only the
/// path to that field is parsed; anything unexpected is `None`.
fn not_after(cert: &[u8]) -> Option<i64> {
    let (_, certificate, _) = element(cert).filter(|(tag, _, _)| *tag == 0x30)?;
    let (_, tbs, _) = element(certificate).filter(|(tag, _, _)| *tag == 0x30)?;
    let mut rest = tbs;
    // [0] version, when present; then serial, signature and issuer.
    if rest.first() == Some(&0xa0) {
        rest = element(rest)?.2;
    }
    for _ in 0..3 {
        rest = element(rest)?.2;
    }
    let (_, validity, _) = element(rest).filter(|(tag, _, _)| *tag == 0x30)?;
    let (_, _, after) = element(validity)?;
    let (tag, time, _) = element(after)?;
    let text = std::str::from_utf8(time).ok()?.strip_suffix('Z')?;
    let (year, rest) = match tag {
        // UTCTime, YYMMDDHHMMSS: 1950 to 2049 (RFC 5280, 4.1.2.5.1).
        0x17 => {
            let two: i64 = text.get(..2)?.parse().ok()?;
            (
                if two < 50 { 2000 + two } else { 1900 + two },
                text.get(2..)?,
            )
        }
        // GeneralizedTime, YYYYMMDDHHMMSS.
        0x18 => (text.get(..4)?.parse().ok()?, text.get(4..)?),
        _ => return None,
    };
    if rest.len() != 10 || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let field = |at: usize| rest.get(at..at + 2)?.parse::<i64>().ok();
    let (month, day) = (field(0)?, field(2)?);
    let (hour, minute, second) = (field(4)?, field(6)?, field(8)?);
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// One DER element: (tag, content, what follows it).
fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let length = rest[..count]
            .iter()
            .fold(0_usize, |length, &byte| (length << 8) | usize::from(byte));
        (length, &rest[count..])
    };
    (rest.len() >= length).then(|| (tag, &rest[..length], &rest[length..]))
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Serve `router` over TLS until `shutdown`, then drain open connections for
/// at most `drain` (`server.graceful_shutdown_secs`).
///
/// # Errors
///
/// None today; the signature matches the plain listener's.
pub async fn serve(
    listener: TcpListener,
    tls: Tls,
    router: Router,
    shutdown: impl Future<Output = ()> + Send,
    drain: Duration,
) -> Result<()> {
    let Tls {
        acceptor,
        current,
        settings,
    } = tls;
    let watcher = tokio::spawn(watch(current, settings));
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
    watcher.abort();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tls")
            .join(name)
    }

    fn end_entity(name: &str) -> Vec<u8> {
        CertificateDer::pem_file_iter(fixture(name))
            .expect("pem")
            .next()
            .expect("a certificate")
            .expect("parse")
            .as_ref()
            .to_vec()
    }

    /// Checked against `openssl x509 -enddate`: a `GeneralizedTime` after
    /// 2049, a `UTCTime` before.
    #[test]
    fn not_after_is_read_from_both_time_forms() {
        assert_eq!(not_after(&end_entity("server.crt")), Some(4_943_677_792));
        assert_eq!(not_after(&end_entity("renewed.crt")), Some(2_420_813_701));
        assert_eq!(not_after(b"\x30\x03\x02\x01\x01"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn a_key_that_is_not_the_certificates_is_refused() {
        let settings = TlsSettings {
            certificate: fixture("server.crt"),
            key: fixture("renewed.key"),
            minimum: MinimumTls::Tls12,
        };
        let error = load(&settings).expect_err("refused");
        assert!(error.to_string().contains("renewed.key"), "{error}");
    }
}
