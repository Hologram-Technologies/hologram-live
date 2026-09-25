//! Two real Hologram daemons converge immutable objects over authenticated membership.

use hologram_live::config::AppConfig;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    _root: tempfile::TempDir,
    port: u16,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(port: u16, seed: Option<u16>, token: &str) -> Server {
    start_without_module(port, seed, token, None)
}

fn start_without_module(
    port: u16,
    seed: Option<u16>,
    token: &str,
    disabled_module: Option<&str>,
) -> Server {
    let root = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.paths.config_dir = root.path().join("config");
    config.paths.data_dir = root.path().join("data");
    config.paths.state_dir = root.path().join("state");
    config.paths.cache_dir = root.path().join("cache");
    config.server.listen = format!("127.0.0.1:{port}");
    config.cluster.advertise_endpoint = Some(format!("http://127.0.0.1:{port}"));
    config.cluster.seeds = seed
        .map(|p| vec![format!("http://127.0.0.1:{p}")])
        .unwrap_or_default();
    config.cluster.heartbeat_interval_secs = 1;
    config.cluster.node_ttl_secs = 3;
    if let Some(disabled_module) = disabled_module {
        config
            .modules
            .enabled
            .retain(|module| module != disabled_module);
    }
    "HOLOGRAM_CLUSTER_E2E_TOKEN".clone_into(&mut config.cluster.token_env);
    std::fs::create_dir_all(&config.paths.config_dir).unwrap();
    let file = config.paths.config_dir.join("live.toml");
    std::fs::write(&file, toml::to_string_pretty(&config).unwrap()).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--config")
        .arg(file)
        .arg("serve")
        .env("HOLOGRAM_CLUSTER_E2E_TOKEN", token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    Server {
        child,
        _root: root,
        port,
    }
}

#[test]
fn placement_selects_the_peer_that_advertises_the_operation() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let first = start_without_module(port(), None, token, Some("dev.hologram.live.chat"));
    let second = start(port(), Some(first.port), token);
    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    let expected_endpoint = format!("http://127.0.0.1:{}", second.port);
    loop {
        let selected = client
            .get(format!(
                "http://127.0.0.1:{}/api/v1/nodes/placement",
                first.port
            ))
            .query(&[("resource", "conversation:e2e"), ("operation", "chat.send")])
            .send()
            .ok()
            .and_then(|response| response.error_for_status().ok())
            .and_then(|response| response.json::<serde_json::Value>().ok());
        if selected.as_ref().and_then(|node| node["endpoint"].as_str())
            == Some(expected_endpoint.as_str())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "capable peer was never selected: {selected:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Ownership must mean the same thing on both sides of a join, not just from
/// the seed that happens to be the one everyone queries.
///
/// `TokenAdmission` is authenticated in only one direction by construction: a
/// joiner proves itself to the seed with a signed, ticket-bearing request,
/// but a join *response* carries no signature, so nothing symmetric happens
/// automatically. Before admission was made symmetric (pinning the peer
/// identity a successful join response claims) and the local node was
/// trusted for itself, this exact scenario reproduced two distinct failures
/// that `placement_selects_the_peer_that_advertises_the_operation` could not
/// see because it only ever queries the seed: the joiner's own admitted set
/// stayed empty forever, so it could place nothing at all — not even
/// operations it advertises itself — and separately, neither side ever
/// trusted its own identity, so a node could never be selected as owner by
/// its own reckoning regardless of the rendezvous hash.
#[test]
fn placement_agrees_from_both_sides_of_the_cluster() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let first = start_without_module(port(), None, token, Some("dev.hologram.live.chat"));
    let second = start(port(), Some(first.port), token);
    let client = reqwest::blocking::Client::new();
    let expected_endpoint = format!("http://127.0.0.1:{}", second.port);

    let placement = |queried_port: u16| {
        client
            .get(format!(
                "http://127.0.0.1:{queried_port}/api/v1/nodes/placement"
            ))
            .query(&[("resource", "conversation:e2e"), ("operation", "chat.send")])
            .send()
            .ok()
            .and_then(|response| response.error_for_status().ok())
            .and_then(|response| response.json::<serde_json::Value>().ok())
    };

    // Wait for the seed's view to converge first, exactly as the sibling test
    // does — this only proves the *seed* can name the capable peer.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let selected = placement(first.port);
        if selected.as_ref().and_then(|node| node["endpoint"].as_str())
            == Some(expected_endpoint.as_str())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "capable peer was never selected from the seed: {selected:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Now ask the *joiner* the same question about itself. Symmetric
    // admission needs one more round trip than the seed's own view (the
    // joiner learns to trust the seed only once it has processed a join
    // response), so this polls independently rather than asserting once.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let selected = placement(second.port);
        if selected.as_ref().and_then(|node| node["endpoint"].as_str())
            == Some(expected_endpoint.as_str())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the joiner could not name itself as the capable peer for its own operation: {selected:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn authenticated_peers_replicate_an_immutable_object() {
    hologram_live::util::install_crypto_provider();
    let first = start(
        port(),
        None,
        "a sufficiently long shared cluster test token",
    );
    let client = reqwest::blocking::Client::new();
    let metadata: serde_json::Value = client
        .post(format!("http://127.0.0.1:{}/api/v1/objects", first.port))
        .header("content-type", "text/plain")
        .header("x-hologram-kind", "file")
        .body("cluster payload")
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    let id = metadata["id"].as_str().unwrap();
    let second = start(
        port(),
        Some(first.port),
        "a sufficiently long shared cluster test token",
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if client
            .get(format!(
                "http://127.0.0.1:{}/api/v1/objects/{id}",
                second.port
            ))
            .send()
            .ok()
            .and_then(|response| response.bytes().ok())
            .as_deref()
            == Some(b"cluster payload")
        {
            break;
        }
        assert!(Instant::now() < deadline, "object never converged");
        std::thread::sleep(Duration::from_millis(100));
    }
}
