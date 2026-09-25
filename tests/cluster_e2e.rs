//! Two real Hologram daemons converge immutable objects over authenticated membership.

use hologram_live::config::AppConfig;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Ceiling for every polling loop in this file that waits on eventual
/// convergence (a node directory settling, bidirectional admission landing,
/// an object replicating) as well as the one that waits for a daemon's
/// listen socket to come up.
///
/// These loops return the instant their condition holds, so a generous
/// ceiling costs nothing on the happy path — it only bounds how long a
/// genuine failure takes to report. Normal convergence in this suite takes
/// about 4 seconds; a serialized full-workspace run puts other test
/// binaries' daemons and subprocesses on the same CPU, which can eat far
/// more than the 15s this used to be, producing a false failure under no
/// real defect. 60 seconds is >14x the ~4s happy path, comfortably above
/// anything workspace-level contention has been observed to cost, while
/// still surfacing a truly broken mechanism well inside a test-run timeout.
///
/// Not used for negative assertions (a fixed window in which something
/// must *not* happen) — see `a_node_without_the_admission_secret_is_refused`,
/// where widening the window would only make the test slower, not stronger.
const CONVERGENCE_DEADLINE: Duration = Duration::from_mins(1);

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
        loopback_endpoints(seed),
        token,
        disabled_module,
    )
}

/// A daemon with more than one configured seed, which
/// `a_join_reply_cannot_install_a_record_for_another_identity` needs: it points
/// one node at both a real peer and a hostile responder.
fn start_with_seeds(port: u16, seeds: Vec<String>, token: &str) -> Server {
    start_in(tempfile::tempdir().unwrap(), port, seeds, token, None)
}

fn loopback_endpoints(seed: Option<u16>) -> Vec<String> {
    seed.map(|p| vec![format!("http://127.0.0.1:{p}")])
        .unwrap_or_default()
}

/// Restarts a daemon on an existing state directory with **no** configured
/// seeds, so the only thing it can rejoin from is what it persisted.
fn restart_without_seeds(root: tempfile::TempDir, port: u16, token: &str) -> Server {
    start_in(root, port, Vec::new(), token, None)
}

/// The one place a daemon is actually spawned. `root` is a parameter rather
/// than created here so a state directory can outlive one process, which is
/// what `a_restarted_node_rejoins_without_any_configured_seed` needs.
fn start_in(
    root: tempfile::TempDir,
    port: u16,
    seeds: Vec<String>,
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
    config.cluster.seeds = seeds;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
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

/// The records `port`'s node directory currently lists, as `(node_id,
/// endpoint)` pairs, or `None` if it could not be asked.
fn directory(client: &reqwest::blocking::Client, port: u16) -> Option<Vec<(String, String)>> {
    client
        .get(format!("http://127.0.0.1:{port}/api/v1/nodes"))
        .send()
        .ok()
        .and_then(|response| response.error_for_status().ok())
        .and_then(|response| response.json::<Vec<serde_json::Value>>().ok())
        .map(|records| {
            records
                .into_iter()
                .filter_map(|record| {
                    Some((
                        record["node_id"].as_str()?.to_owned(),
                        record["endpoint"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
}

/// Answers every connection with one fixed HTTP response, forever, and reports
/// the port it bound.
///
/// `body` is built *from* that port, because the reply this test needs has to
/// name the responder's own origin, which is only known once the socket is
/// bound.
///
/// Deliberately hand-rolled rather than an axum stub: the whole point is a
/// responder that ignores the signed request entirely and answers with
/// something a well-behaved peer never would.
fn spawn_fixed_responder(body: impl FnOnce(u16) -> String) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = body(port);
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\
         content-length: {}\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let response = response.clone();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                // Drain the request head so the caller's write completes before
                // the reply closes the connection. One read is normally the
                // whole of it; the loop covers a segmented one.
                let mut scratch = [0_u8; 4096];
                let mut seen: Vec<u8> = Vec::new();
                while let Ok(read) = stream.read(&mut scratch) {
                    if read == 0 {
                        break;
                    }
                    seen.extend_from_slice(&scratch[..read]);
                    if seen.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    port
}

/// Final review, FIX 1: a join *reply* is unsigned, so nothing in it proves who
/// sent it. The node directory is keyed by `node_id` and a write replaces the
/// whole record, so persisting a reply would let any origin this node dials
/// install an already-admitted member's `node_id` against the *attacker's*
/// endpoint — and `/api/v1/nodes/owner` and `/api/v1/nodes/placement` hand out
/// whatever endpoint the directory holds for an admitted identity, including
/// for inference and Holo placement.
///
/// The shape of this test matters. A hostile responder alone proves little
/// while the impersonated node is also being dialled: its own reply carries its
/// real endpoint and lands in the same round, so the overwrite is corrected
/// within microseconds and a sampling test sees almost nothing. So the victim
/// is taken down once its authenticated record has arrived. From that moment
/// the only thing still claiming its identity is the responder, and the two
/// behaviours diverge completely: with the reply write deleted the victim's
/// record simply ages out, while with it reinstated the responder *keeps the
/// record alive at its own endpoint*, refreshing `last_seen` every round so it
/// never prunes at all.
///
/// So the assertion is in two parts — the victim's id must never be seen at the
/// responder's endpoint, and once the victim is gone its record must disappear
/// rather than be kept alive by a stranger.
#[test]
fn a_join_reply_cannot_install_a_record_for_another_identity() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let victim = start(port(), None, token);
    let client = reqwest::blocking::Client::new();

    // `victim` heartbeats its own record, so its directory names its identity.
    let victim_endpoint = format!("http://127.0.0.1:{}", victim.port);
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
    let victim_node_id = loop {
        if let Some(found) = directory(&client, victim.port)
            .unwrap_or_default()
            .into_iter()
            .find(|(_, endpoint)| endpoint == &victim_endpoint)
            .map(|(node_id, _)| node_id)
        {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "the victim never published its own record"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(victim_node_id.starts_with("ed25519:"), "{victim_node_id}");

    // The hostile responder answers every join with the victim's identity
    // against its own origin.
    let claimed = victim_node_id.clone();
    let rogue_port = spawn_fixed_responder(move |port| {
        serde_json::json!({
            "node": {
                "node_id": claimed,
                "version": "1.0.0",
                "operations": ["nodes.list"],
                "endpoint": format!("http://127.0.0.1:{port}"),
                "last_seen_millis": 0
            },
            "peers": []
        })
        .to_string()
    });
    let rogue_endpoint = format!("http://127.0.0.1:{rogue_port}");

    let node = start_with_seeds(
        port(),
        vec![victim_endpoint.clone(), rogue_endpoint.clone()],
        token,
    );

    // The authenticated path still populates the directory: `node` dials the
    // victim, the victim notices `node` in its own directory and dials back, and
    // that inbound join carries a record `record_matches_signer` has checked
    // against its signature.
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
    loop {
        let records = directory(&client, node.port).unwrap_or_default();
        if records.contains(&(victim_node_id.clone(), victim_endpoint.clone())) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the victim's authenticated record never reached the directory: {records:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // With the victim gone, nothing legitimate refreshes its record. Only the
    // responder still claims its identity.
    drop(victim);
    let deadline = Instant::now() + CONVERGENCE_DEADLINE;
    loop {
        let records = directory(&client, node.port).unwrap_or_default();
        assert!(
            !records.contains(&(victim_node_id.clone(), rogue_endpoint.clone())),
            "an unsigned join reply installed an admitted identity at the responder's endpoint: \
             {records:?}"
        );
        if !records
            .iter()
            .any(|(node_id, _)| node_id == &victim_node_id)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "a dead node's record was kept alive by a stranger's unsigned reply: {records:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Final review, FIX 3: both cluster object routes are mounted on the *public*
/// router (`src/server.rs`), so their own `authorize_cluster_request` call is
/// the only thing standing between the object inventory — and every object
/// body — and the open internet. Deleting either call left the whole test suite
/// green before this.
#[test]
fn the_cluster_object_routes_refuse_an_unsigned_request() {
    hologram_live::util::install_crypto_provider();
    let node = start(
        port(),
        None,
        "a sufficiently long shared cluster test token",
    );
    let client = reqwest::blocking::Client::new();
    let metadata: serde_json::Value = client
        .post(format!("http://127.0.0.1:{}/api/v1/objects", node.port))
        .header("content-type", "text/plain")
        .header("x-hologram-kind", "file")
        .body("a body no unsigned caller may read")
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    let id = metadata["id"].as_str().unwrap();

    for path in [
        "/api/v1/cluster/objects".to_owned(),
        format!("/api/v1/cluster/objects/{id}"),
    ] {
        // No proof headers at all.
        let response = client
            .get(format!("http://127.0.0.1:{}{path}", node.port))
            .send()
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{path} answered an unsigned request"
        );
        let body = response.text().unwrap_or_default();
        assert!(
            !body.contains("a body no unsigned caller may read"),
            "{path} leaked an object body: {body}"
        );

        // Present but invalid: a well-formed set of headers whose signature is
        // not one, so the route cannot pass by merely finding the headers.
        let response = client
            .get(format!("http://127.0.0.1:{}{path}", node.port))
            .header(
                "x-hologram-cluster-node",
                "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            )
            .header("x-hologram-cluster-timestamp", "1700000000000")
            .header("x-hologram-cluster-signature", "00".repeat(64))
            .header(
                "x-hologram-cluster-recipient",
                format!("http://127.0.0.1:{}", node.port),
            )
            .send()
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{path} answered an invalidly signed request"
        );
    }
}
