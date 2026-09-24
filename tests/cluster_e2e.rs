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
