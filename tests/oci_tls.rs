#![cfg(feature = "oci")]
//! TLS from the reference's settings (`http.tls.*`), measured against the
//! phase's own acceptance checks (P6 T2): a stalled handshake never delays
//! the next client, ALPN offers HTTP/2 and HTTP/1.1, a minimum version
//! refuses older clients, and plain HTTP on the TLS port fails closed.

use hologram_live::config::{TlsConfig, TlsVersion};
use hologram_live::tls;
use rustls::pki_types::{CertificateDer, ServerName};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A self-signed pair for `localhost` and `127.0.0.1`, written where the
/// configuration points, with the DER kept for the clients' root of trust.
struct Pair {
    _dir: tempfile::TempDir,
    certificate: std::path::PathBuf,
    key: std::path::PathBuf,
    certificate_der: CertificateDer<'static>,
}

fn pair() -> Pair {
    let dir = tempfile::tempdir().expect("tempdir");
    let key_pair = rcgen::KeyPair::generate().expect("key pair");
    let mut params =
        rcgen::CertificateParams::new(vec!["localhost".to_owned()]).expect("certificate params");
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(Ipv4Addr::LOCALHOST.into()));
    let certificate = params
        .self_signed(&key_pair)
        .expect("self-signed certificate");
    let certificate_path = dir.path().join("domain.crt");
    let key_path = dir.path().join("domain.key");
    std::fs::write(&certificate_path, certificate.pem()).expect("certificate file");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("key file");
    Pair {
        _dir: dir,
        certificate: certificate_path,
        key: key_path,
        certificate_der: certificate.der().clone(),
    }
}

impl Pair {
    fn config(&self, minimum: TlsVersion) -> TlsConfig {
        TlsConfig {
            certificate: self.certificate.clone(),
            key: self.key.clone(),
            minimum,
        }
    }
}

struct Server {
    addr: SocketAddr,
    pair: Pair,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(minimum: TlsVersion) -> Server {
    let pair = pair();
    let acceptor = tls::build(&pair.config(minimum)).expect("acceptor");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    let router = axum::Router::new().route(
        "/v2/",
        axum::routing::get(|| async { axum::Json(serde_json::json!({})) }),
    );
    let task = tokio::spawn(async move {
        tls::serve(listener, acceptor, router, std::future::pending())
            .await
            .expect("serve TLS");
    });
    Server { addr, pair, task }
}

/// A raw TLS client speaking HTTP/1.1 by hand: the response is one readable
/// buffer, and the negotiation is visible on the stream. Handshake failures
/// come back as errors, so a refusal can be told from a success.
async fn connect(
    server: &Server,
    versions: &'static [&'static rustls::SupportedProtocolVersion],
    alpn: Option<&[u8]>,
) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(server.pair.certificate_der.clone())
        .expect("root certificate");
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .expect("client protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    if let Some(alpn) = alpn {
        config.alpn_protocols = vec![alpn.to_vec()];
    }
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let stream = tokio::net::TcpStream::connect(server.addr).await?;
    let name = ServerName::IpAddress(Ipv4Addr::LOCALHOST.into());
    connector.connect(name, stream).await
}

async fn get_v2(
    stream: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"GET /v2/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("response");
    let response = String::from_utf8(response).expect("utf-8");
    let status: u16 = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (status, response)
}

/// A client that opens the socket and never speaks: the exact stall the
/// accept loop must survive.
async fn stall(server: &Server) -> tokio::net::TcpStream {
    tokio::net::TcpStream::connect(server.addr)
        .await
        .expect("stalled client")
}

#[tokio::test(flavor = "multi_thread")]
async fn http11_is_served_over_the_negotiated_connection() {
    let server = server(TlsVersion::Tls12).await;
    let mut stream = connect(&server, rustls::ALL_VERSIONS, None)
        .await
        .expect("TLS handshake");
    assert_eq!(
        stream.get_ref().1.alpn_protocol(),
        None,
        "a client offering nothing selects nothing"
    );
    let (status, body) = get_v2(&mut stream).await;
    assert_eq!(status, 200);
    assert!(body.contains("{}"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn http2_is_offered_by_alpn() {
    let server = server(TlsVersion::Tls12).await;
    let mut stream = connect(&server, rustls::ALL_VERSIONS, Some(b"h2"))
        .await
        .expect("TLS handshake");
    assert_eq!(
        stream.get_ref().1.alpn_protocol(),
        Some(b"h2".as_slice()),
        "the server offers HTTP/2 first"
    );
    // An h2-negotiated stream handed HTTP/1.1 bytes is a protocol error, so
    // the request is sent through a real h2 client (see `h2_serves_the_api`).
    let _ = get_v2(&mut stream).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn h2_serves_the_api() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let server = server(TlsVersion::Tls12).await;
    let certificate = reqwest::Certificate::from_der(server.pair.certificate_der.as_ref())
        .expect("root certificate");
    let client = reqwest::Client::builder()
        .add_root_certificate(certificate)
        .use_rustls_tls()
        .build()
        .expect("client");
    let response = client
        .get(format!("https://127.0.0.1:{}/v2/", server.addr.port()))
        .send()
        .await
        .expect("response");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        format!("{:?}", response.version()).contains("HTTP/2"),
        "ALPN negotiated h2, {:?}",
        response.version()
    );
    let body: serde_json::Value = response.json().await.expect("body");
    assert_eq!(body, serde_json::json!({}));
}

/// The phase's own acceptance check: 200 stalled handshakes must not delay
/// the 201st client past one second.
#[tokio::test(flavor = "multi_thread")]
async fn two_hundred_stalled_handshakes_do_not_delay_the_next_client() {
    let server = server(TlsVersion::Tls12).await;
    let stalled: Vec<tokio::net::TcpStream> = {
        let mut stalled = Vec::new();
        for _ in 0..200 {
            stalled.push(stall(&server).await);
        }
        stalled
    };
    let started = std::time::Instant::now();
    let served = tokio::time::timeout(Duration::from_secs(1), async {
        let mut stream = connect(&server, rustls::ALL_VERSIONS, None)
            .await
            .expect("TLS handshake");
        let (status, body) = get_v2(&mut stream).await;
        assert_eq!(status, 200);
        assert!(body.contains("{}"), "{body}");
    })
    .await;
    served.expect("the 201st connection is served within one second");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the deadline is the assertion"
    );
    drop(stalled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tls13_minimum_refuses_a_tls12_client() {
    let server = server(TlsVersion::Tls13).await;
    const TLS12_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS12];
    let result =
        tokio::time::timeout(Duration::from_secs(5), connect(&server, TLS12_ONLY, None)).await;
    match result {
        Err(_) => panic!("the refused handshake did not fail within five seconds"),
        Ok(Err(_)) => {}
        Ok(Ok(_)) => panic!("a TLS 1.2 client connected to a TLS 1.3 minimum"),
    }
    // A TLS 1.3 client still works against the same server.
    let mut stream = connect(&server, rustls::ALL_VERSIONS, None)
        .await
        .expect("TLS handshake");
    let (status, _) = get_v2(&mut stream).await;
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_on_the_tls_port_fails_closed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = server(TlsVersion::Tls12).await;
    let mut stream = stall(&server).await;
    stream
        .write_all(b"GET /v2/ HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("plain bytes sent");
    let mut answer = Vec::new();
    // The handshake fails and the socket closes: an error or an empty read,
    // never an answer that pretends the port is plain HTTP.
    let _ = stream.read_to_end(&mut answer).await;
    let body = String::from_utf8_lossy(&answer);
    assert!(
        !body.contains("HTTP/"),
        "a plain-HTTP client must not be answered over a TLS port: {body}"
    );
}

#[test]
fn a_missing_certificate_stops_the_start_naming_the_file() {
    let config = TlsConfig {
        certificate: std::path::PathBuf::from("/nowhere/domain.crt"),
        key: std::path::PathBuf::from("/nowhere/domain.key"),
        minimum: TlsVersion::Tls12,
    };
    let error = match tls::build(&config) {
        Ok(_) => panic!("a missing certificate started"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("/nowhere/domain.crt"));
}

#[test]
fn a_mismatched_pair_is_refused_at_start() {
    let first = pair();
    let second = pair();
    let config = TlsConfig {
        certificate: first.certificate.clone(),
        key: second.key.clone(),
        minimum: TlsVersion::Tls12,
    };
    let error = match tls::build(&config) {
        Ok(_) => panic!("a mismatched pair started"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("do not match") || error.to_string().contains("registry TLS")
    );
}

#[test]
fn garbage_in_a_certificate_file_is_refused_at_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let certificate = dir.path().join("domain.crt");
    let key = dir.path().join("domain.key");
    std::fs::write(&certificate, "not a certificate").expect("write");
    std::fs::write(&key, "not a key").expect("write");
    let config = TlsConfig {
        certificate,
        key,
        minimum: TlsVersion::Tls12,
    };
    assert!(tls::build(&config).is_err(), "garbage is refused");
}
