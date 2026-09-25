//! Two real Hologram daemons converge immutable objects over authenticated membership.

use hologram_live::config::AppConfig;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    /// `None` only after [`Server::stop_keeping_state`] has handed the state
    /// directory to a caller that wants to outlive this process.
    root: Option<tempfile::TempDir>,
    port: u16,
}
impl Server {
    /// Stops this daemon and hands back its state directory, so a second
    /// process can be started on the same root.
    ///
    /// `TempDir` deletes its directory when it drops, and `Server` owns it, so
    /// a restart test cannot simply drop the server: the persisted
    /// `nodes.json`, `node.key` and `cluster-pinned.json` it means to restart
    /// against would go with it.
    fn stop_keeping_state(mut self) -> tempfile::TempDir {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.root
            .take()
            .expect("a server's state directory is taken at most once")
    }
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
    start_in(
        tempfile::tempdir().unwrap(),
        port,
        seed,
        token,
        disabled_module,
    )
}

/// Restarts a daemon on an existing state directory with **no** configured
/// seeds, so the only thing it can rejoin from is what it persisted.
fn restart_without_seeds(root: tempfile::TempDir, port: u16, token: &str) -> Server {
    start_in(root, port, None, token, None)
}

/// The one place a daemon is actually spawned. `root` is a parameter rather
/// than created here so a state directory can outlive one process, which is
/// what `a_restarted_node_rejoins_without_any_configured_seed` needs.
fn start_in(
    root: tempfile::TempDir,
    port: u16,
    seed: Option<u16>,
    token: &str,
    disabled_module: Option<&str>,
) -> Server {
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
        root: Some(root),
        port,
    }
}

/// The number of records `port`'s node directory currently lists, or `None`
/// if it could not be asked.
fn peer_count(client: &reqwest::blocking::Client, port: u16) -> Option<usize> {
    client
        .get(format!("http://127.0.0.1:{port}/api/v1/nodes"))
        .send()
        .ok()
        .and_then(|response| response.error_for_status().ok())
        .and_then(|response| response.json::<Vec<serde_json::Value>>().ok())
        .map(|peers| peers.len())
}

/// Polls `port`'s node directory until it holds exactly `expected` records.
fn await_peer_count(client: &reqwest::blocking::Client, port: u16, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = peer_count(client, port);
        if count == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the node on port {port} never reached {expected} directory records; last saw {count:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
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

/// Defect 8: a node restarted with **no** configured seeds must rejoin from the
/// membership it persisted, not sit isolated with a directory full of peers it
/// never dials.
///
/// The shape of this test matters, because the naive version proves nothing.
/// `nodes.json` is reloaded into the node directory at startup, so a freshly
/// restarted node answers `/api/v1/nodes` with its old peers immediately —
/// even with the peer set left empty, which is exactly the defect. Worse, the
/// seed would normally notice the restarted node in *its* directory and dial it
/// back, so convergence could happen without the restarted node ever taking
/// the initiative.
///
/// So this waits for the seed to prune the dead node out of its own directory
/// first. That same prune feeds `prune_evictions`, which drops the endpoint
/// from the seed's peer table, and the seed here has no configured seeds of its
/// own to fall back on — after that point nothing on the seed's side can
/// re-establish contact. The seed's directory returning to two records
/// therefore proves the restarted node dialled *out*, and the only address it
/// could have dialled came off its own disk.
#[test]
fn a_restarted_node_rejoins_without_any_configured_seed() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let first = start(port(), None, token);
    let second_port = port();
    let second = start(second_port, Some(first.port), token);

    let client = reqwest::blocking::Client::new();
    await_peer_count(&client, first.port, 2);

    // Take the joiner down, keeping its state directory: `node.key` (so it
    // restarts under the same identity), `cluster-pinned.json` (so admission
    // survives) and `nodes.json` (the persisted membership under test).
    let root = second.stop_keeping_state();
    await_peer_count(&client, first.port, 1);

    let restarted = restart_without_seeds(root, second_port, token);
    await_peer_count(&client, first.port, 2);
    await_peer_count(&client, restarted.port, 2);
}

/// Defect 3: holding a *different* secret is not membership. A node whose
/// admission ticket does not verify never enters the directory at all.
///
/// Task 7 established this by hand with three daemons — the rogue was refused
/// at join with `403`, never appeared in either legitimate node's directory and
/// was never pinned. This commits that as a test, and asserts it in both
/// directions: the seed never records the intruder, and the intruder never
/// records the seed, because a refused join returns no `ClusterJoinResponse`
/// for it to learn from.
///
/// Both nodes are observed over several heartbeat rounds rather than sampled
/// once, so a join that is merely slow to be accepted would still fail this.
#[test]
fn a_node_without_the_admission_secret_is_refused() {
    hologram_live::util::install_crypto_provider();
    let first = start(
        port(),
        None,
        "a sufficiently long shared cluster test token",
    );
    let intruder = start(
        port(),
        Some(first.port),
        "an entirely different long secret value",
    );

    let client = reqwest::blocking::Client::new();
    // `heartbeat_interval_secs` is 1 in this harness, so this observes on the
    // order of five join attempts.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        assert_eq!(
            peer_count(&client, first.port),
            Some(1),
            "the intruder must never appear in the seed's directory"
        );
        assert_eq!(
            peer_count(&client, intruder.port),
            Some(1),
            "a refused joiner learns nothing about the node that refused it"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
