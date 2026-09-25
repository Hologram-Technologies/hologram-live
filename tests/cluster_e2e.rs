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
    // Anti-entropy is now decoupled from the heartbeat (default 60s); keep it
    // fast here so replication still fires within these tests' deadlines.
    config.cluster.replication_interval_secs = 1;
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

/// A joiner must be able to place its own advertised operation on itself,
/// not just have the seed place it on the joiner.
///
/// This exercises self-trust only (`AppState::admitted_with_self`): every
/// node trusts its own identity for ownership purposes, since nothing
/// authenticates a node to itself. Before that existed, a node's own id was
/// never in its own admitted set, so
/// `placement_selects_the_peer_that_advertises_the_operation` could not see
/// the failure — it only ever queries the seed, which happened to defer to
/// the joiner regardless. Querying the *joiner* about itself surfaces it
/// directly: without self-trust this returns 404, because the joiner's own
/// id is excluded from the only candidate set it can see.
///
/// This test does **not** exercise bidirectional admission between two
/// distinct nodes — see `the_joiner_comes_to_admit_the_seed` for that, which
/// this test cannot substitute for: `second` is the only node advertising
/// `chat.send` here, so the joiner can answer this correctly by trusting
/// only itself, whether or not it has ever admitted the seed.
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

    // Now ask the *joiner* the same question about itself. This needs only
    // self-trust (the joiner's own id in its own admitted set), which is
    // present from the moment `AppState` is built, but still polls rather
    // than asserting once immediately after the seed's view converges above —
    // the two servers may not have reached that instant with identical
    // timing, and this keeps the test from being sensitive to that.
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

/// Pins the bidirectional-admission mechanism itself: the joiner must come to
/// admit the *seed*, not just itself.
///
/// `TokenAdmission` is authenticated in only one direction by construction: a
/// joiner proves itself to the seed with a signed, ticket-bearing request,
/// but a join *response* carries no signature, so nothing symmetric happens
/// automatically. `run` closes that by re-seeding its peer table from the
/// node directory every round (not just once at startup), so the seed
/// eventually notices the joiner in its own directory and dials it back,
/// presenting a real ticket the same way any node proves itself — at which
/// point the joiner admits the seed exactly as the seed admitted the joiner.
///
/// This is queried with `operation=nodes.list`, which every node advertises,
/// specifically so the answer is never forced by which side happens to
/// support the capability — the whole point is to observe whether the
/// *joiner's own admitted set* has come to include the seed. It samples for
/// a resource key that the seed assigns to *itself*, then asks the joiner
/// the identical question. `owner_for_operation` returns the seed only if
/// the seed survives the joiner's own `admitted.contains` filter: if the
/// joiner had not admitted the seed, this would return the joiner's own
/// endpoint instead (its only remaining candidate, via self-trust), not the
/// seed's — which is exactly the failure this test is written to catch, and
/// which `placement_agrees_from_both_sides_of_the_cluster` cannot, since that
/// test's queried operation is only ever advertised by the joiner itself.
#[test]
fn the_joiner_comes_to_admit_the_seed() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let first = start(port(), None, token);
    let second = start(port(), Some(first.port), token);
    let client = reqwest::blocking::Client::new();
    let first_endpoint = format!("http://127.0.0.1:{}", first.port);

    let owner_endpoint = |queried_port: u16, resource: &str| {
        client
            .get(format!(
                "http://127.0.0.1:{queried_port}/api/v1/nodes/placement"
            ))
            .query(&[("resource", resource), ("operation", "nodes.list")])
            .send()
            .ok()
            .and_then(|response| response.error_for_status().ok())
            .and_then(|response| response.json::<serde_json::Value>().ok())
            .and_then(|node| node["endpoint"].as_str().map(str::to_owned))
    };

    // Find a resource key the seed assigns to *itself* — guaranteed to turn
    // up quickly among a handful of samples, since a node always trusts its
    // own identity for at least some share of the rendezvous-hash space.
    let deadline = Instant::now() + Duration::from_secs(15);
    let resource = loop {
        if let Some(found) = (0..64)
            .map(|index| format!("bidirectional-admission-check:{index}"))
            .find(|resource| {
                owner_endpoint(first.port, resource).as_deref() == Some(first_endpoint.as_str())
            })
        {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "the seed never named itself owner of any sampled key"
        );
        std::thread::sleep(Duration::from_millis(100));
    };

    // The joiner must answer the identical question about the identical key
    // with the seed's endpoint too, once bidirectional admission has had time
    // to converge.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if owner_endpoint(second.port, &resource).as_deref() == Some(first_endpoint.as_str()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the joiner never came to admit the seed: querying the joiner for a key the seed \
             assigns to itself did not return the seed's endpoint"
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
