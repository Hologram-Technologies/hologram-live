#![cfg(feature = "oci")]
//! TLS on the registry's listener (plan P6 T2), through the real binary
//! started as the reference is: `serve --registry-config config.yml` with
//! `http.tls`. Certificates are the throwaway ones in `tests/fixtures/tls`.

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

fn slash(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

struct Server {
    child: Child,
    port: u16,
    _root: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `registry serve config.yml` with `http.tls`; `extra` is more YAML under `tls:`.
fn start(extra: &str) -> Server {
    let root = tempfile::tempdir().expect("tempdir");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
        .port();
    let file = root.path().join("config.yml");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{port}\n  tls:\n    certificate: {}\n    key: {}\n{extra}",
            slash(&root.path().join("volume")),
            slash(&fixture("chain.crt")),
            slash(&fixture("server.key")),
        ),
    )
    .expect("config.yml");
    let child = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("serve")
        .arg("--registry-config")
        .arg(&file)
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .env("HOLOGRAM_CONFIG_DIR", root.path().join("config"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let server = Server {
        child,
        port,
        _root: root,
    };
    let deadline = Instant::now() + Duration::from_mins(1);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the registry did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    server
}

fn client(
    versions: &[&'static rustls::SupportedProtocolVersion],
    alpn: &[&[u8]],
) -> Arc<rustls::ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_file(fixture("ca.crt")).expect("ca.crt"))
        .expect("the test CA");
    let mut config = rustls::ClientConfig::builder_with_protocol_versions(versions)
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
    Arc::new(config)
}

fn connect(
    port: u16,
    config: Arc<rustls::ClientConfig>,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let name = ServerName::try_from("localhost").expect("name");
    let connection = rustls::ClientConnection::new(config, name).expect("client");
    rustls::StreamOwned::new(connection, tcp)
}

/// One HTTP/1.1 exchange over TLS: the status and the head.
fn exchange(port: u16, method: &str, path: &str) -> (u16, String) {
    let mut stream = connect(
        port,
        client(&[&rustls::version::TLS13, &rustls::version::TLS12], &[]),
    );
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").expect("send");
    let mut raw = Vec::new();
    // rustls reports a peer that closes without close_notify as an error; the
    // answer is complete by then.
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let head = text.split("\r\n\r\n").next().unwrap_or_default().to_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status");
    (status, head)
}

#[test]
fn over_tls_the_registry_answers_and_its_locations_are_https() {
    let server = start("");
    let (status, _) = exchange(server.port, "GET", "/v2/");
    assert_eq!(status, 200);
    let (status, head) = exchange(server.port, "POST", "/v2/team/app/blobs/uploads/");
    assert_eq!(status, 202, "{head}");
    let location = head
        .lines()
        .find_map(|line| {
            line.strip_prefix("Location: ")
                .or_else(|| line.strip_prefix("location: "))
        })
        .expect("a Location");
    assert!(
        location.starts_with(&format!(
            "https://localhost:{}/v2/team/app/blobs/uploads/",
            server.port
        )),
        "{location}"
    );
}

#[test]
fn the_whole_chain_is_sent_and_h2_is_offered() {
    let server = start("");
    let mut stream = connect(
        server.port,
        client(&[&rustls::version::TLS13], &[b"h2", b"http/1.1"]),
    );
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .expect("handshake");
    }
    assert_eq!(stream.conn.alpn_protocol(), Some(&b"h2"[..]));
    let chain = stream.conn.peer_certificates().expect("certificates");
    assert_eq!(chain.len(), 2, "the certificate and the CA from chain.crt");
}

#[test]
fn a_plain_request_on_the_tls_port_gets_gos_answer() {
    let server = start("");
    let mut tcp = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    tcp.write_all(b"GET /v2/ HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("send");
    let mut answer = Vec::new();
    let _ = tcp.read_to_end(&mut answer);
    assert_eq!(
        String::from_utf8_lossy(&answer),
        "HTTP/1.0 400 Bad Request\r\n\r\nClient sent an HTTP request to an HTTPS server.\n"
    );
}

#[test]
fn clients_that_never_finish_the_handshake_do_not_hold_up_the_next() {
    let server = start("");
    let stalled: Vec<TcpStream> = (0..200)
        .map(|_| TcpStream::connect(("127.0.0.1", server.port)).expect("connect"))
        .collect();
    let started = Instant::now();
    let (status, _) = exchange(server.port, "GET", "/v2/");
    assert_eq!(status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "served in {:?} beside 200 stalled handshakes",
        started.elapsed()
    );
    drop(stalled);
}

#[test]
fn minimumtls_13_refuses_a_tls_12_client() {
    let server = start("    minimumtls: tls1.3\n");
    let mut old = connect(server.port, client(&[&rustls::version::TLS12], &[]));
    let mut failed = false;
    while old.conn.is_handshaking() {
        if old.conn.complete_io(&mut old.sock).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "a TLS 1.2 client was accepted");
    let (status, _) = exchange(server.port, "GET", "/v2/");
    assert_eq!(status, 200, "a TLS 1.3 client still is");
}
