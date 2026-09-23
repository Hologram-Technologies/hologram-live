#![cfg(feature = "oci")]
//! The Hologram provider publishing into a registry that is not kappa-registry
//! (plan P8 T5, FR-022): blobs go through the registry route every registry
//! has, and a password file is what it logs in with.
//!
//! The registry under test is this binary, started as the reference is, with
//! `auth.htpasswd`. The provider is `KappaClient`, the same code the hub's
//! daily publish uses.

use hologram_live::artifact_pull::LayerFetch;
use hologram_live::artifact_push::LayerPublish;
use hologram_live::config::RegistryConfig;
use hologram_live::registry::kappa_client::KappaClient;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The gates' own password file: `gate` / `gate-password`.
const USER: &str = "gate";
const PASSWORD: &str = "gate-password";

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

fn start() -> Server {
    let root = tempfile::tempdir().expect("tempdir");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
        .port();
    let passwd = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("apps/registry/gates/differential/fixtures/htpasswd");
    let file = root.path().join("config.yml");
    let slash = |path: &std::path::Path| path.display().to_string().replace('\\', "/");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{port}\nauth:\n  htpasswd:\n    realm: Registry Realm\n    path: {}\n",
            slash(&root.path().join("volume")),
            slash(&passwd),
        ),
    )
    .expect("config.yml");
    let child = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .args(["serve", "--registry-config"])
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

fn provider(port: u16, token: &str) -> KappaClient {
    let config = RegistryConfig {
        provider: "kappa".to_owned(),
        endpoint: format!("http://127.0.0.1:{port}"),
        namespace: "model-hub".to_owned(),
        token: token.to_owned(),
        upload_flow: "standard".to_owned(),
        ..RegistryConfig::default()
    };
    KappaClient::new(&config).expect("client")
}

/// One anonymous HTTP/1.1 request: the status and the body.
fn request(port: u16, method: &str, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").expect("send");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a head");
    let status = String::from_utf8_lossy(&raw[..split])
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status");
    (status, raw[split + 4..].to_vec())
}

/// The publish path the hub runs every day, against a registry that only
/// speaks the distribution API: the blob goes through
/// `POST /v2/<repo>/blobs/uploads/?digest=…`, the login is Basic, and what
/// was published reads back byte for byte with the same login. A password
/// file protects reads as well, as it does in the reference, so anonymous
/// gets the challenge.
#[test]
fn the_provider_publishes_through_the_standard_route_and_logs_in() {
    let server = start();
    let client = provider(server.port, &format!("{USER}:{PASSWORD}"));
    let bytes = b"a layer the hub publishes".to_vec();
    let digest = hologram_live::oci_store::Digest::sha256_of(&bytes)
        .as_str()
        .to_owned();
    LayerPublish::put_blob(
        &client,
        "model-hub/index",
        &digest,
        "application/octet-stream",
        &bytes,
    )
    .expect("the blob goes through the standard upload");

    let manifest = format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.oci.empty.v1+json","digest":"{digest}","size":{}}},"layers":[{{"mediaType":"application/octet-stream","digest":"{digest}","size":{}}}]}}"#,
        bytes.len(),
        bytes.len()
    );
    LayerPublish::put_manifest(&client, "model-hub/index", "v1", manifest.as_bytes())
        .expect("the manifest goes");

    // Read back with the same login.
    let read = LayerFetch::blob(&client, "model-hub/index", &digest).expect("the blob reads back");
    assert_eq!(read, bytes, "the bytes came back");
    let (body, _) = LayerFetch::manifest(&client, "model-hub/index", "v1")
        .expect("resolve")
        .expect("the tag is there");
    assert_eq!(body, manifest.as_bytes(), "the manifest came back");

    // A password file protects reads too, as in the reference.
    let (status, _) = request(server.port, "GET", "/v2/model-hub/index/manifests/v1");
    assert_eq!(status, 401, "anonymous gets the challenge");
}

/// Without the password, a publish is refused, and nothing is written.
#[test]
fn a_publish_without_the_password_is_refused() {
    let server = start();
    let client = provider(server.port, "");
    let error = LayerPublish::put_blob(
        &client,
        "model-hub/index",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "application/octet-stream",
        b"denied",
    )
    .expect_err("anonymous publish is refused");
    assert!(
        matches!(error, hologram_live::error::LiveError::Authentication(_))
            || format!("{error}").contains("401")
            || format!("{error}").contains("UNAUTHORIZED"),
        "{error}"
    );
}
