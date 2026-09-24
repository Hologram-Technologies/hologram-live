#![cfg(feature = "oci")]
//! The six concurrency rules of `contracts/registry-api.md`, one test each,
//! against the real binary on loopback.
//!
//! Each rule says what two clients racing may and may not observe. A test that
//! is flaky here is a finding about the rule, not about the test: the rule is
//! either underspecified or the server does not hold it. Run each fifty times
//! before believing it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
/// Eight clients, as the contract's table describes the races.
const CLIENTS: usize = 8;

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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// A registry with delete enabled, on its own volume, in its own HOME.
fn start() -> Server {
    let root = tempfile::tempdir().expect("tempdir");
    let port = free_port();
    let file = root.path().join("config.yml");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  delete:\n    enabled: true\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{port}\n",
            root.path()
                .join("volume")
                .display()
                .to_string()
                .replace('\\', "/")
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
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the registry did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    server
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// A `Location` as a path this helper can send again: the server may answer
    /// an absolute URL, and `http.relativeurls` decides which.
    fn location(&self) -> String {
        let raw = self.header("location").expect("a Location");
        match raw.find("/v2/") {
            Some(at) => raw[at..].to_owned(),
            None => raw.to_owned(),
        }
    }
}

/// One HTTP/1.1 exchange, `Connection: close`, no client library: the point is
/// to control exactly what goes on the wire and when.
fn send(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("timeout");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("send head");
    stream.write_all(body).expect("send body");
    stream.flush().expect("flush");

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    parse(&raw)
}

/// As [`send`], but `None` when the server answered and closed before the body
/// was fully written, or closed without a complete response. That is legal
/// HTTP — a server may reject a request before reading its body — and it is
/// what a stale `PATCH` sometimes observes.
fn try_send(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Option<Answer> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok()?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).ok()?;
    let sent = stream.write_all(body).is_ok() && stream.flush().is_ok();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    if !sent && raw.is_empty() {
        return None;
    }
    raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    Some(parse(&raw))
}

fn parse(raw: &[u8]) -> Answer {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status");
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Answer {
        status,
        headers,
        body,
    }
}

/// Bytes that do not repeat, so a wrong offset shows as wrong content.
fn blob(seed: &str, len: usize) -> Vec<u8> {
    let mut out = vec![0_u8; len];
    blake3::Hasher::new()
        .update(seed.as_bytes())
        .finalize_xof()
        .fill(&mut out);
    out
}

fn digest_of(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}

/// POST, PATCH, PUT: one blob, the ordinary way a client pushes.
fn push_blob(port: u16, repo: &str, bytes: &[u8]) -> (u16, String) {
    let digest = digest_of(bytes);
    let begin = send(port, "POST", &format!("/v2/{repo}/blobs/uploads/"), &[], b"");
    assert_eq!(begin.status, 202, "POST upload");
    let patch = send(
        port,
        "PATCH",
        &begin.location(),
        &[("Content-Type", "application/octet-stream")],
        bytes,
    );
    assert_eq!(patch.status, 202, "PATCH upload");
    let finish = send(
        port,
        "PUT",
        &format!("{}?digest={digest}", patch.location()),
        &[],
        b"",
    );
    (finish.status, digest)
}

/// A manifest naming one layer, distinct per `tag_seed`.
fn manifest_for(layer: &str, tag_seed: &str) -> Vec<u8> {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_TYPE}","annotations":{{"seed":"{tag_seed}"}},"layers":[{{"mediaType":"application/octet-stream","digest":"{layer}","size":0}}]}}"#
    )
    .into_bytes()
}

fn put_manifest(port: u16, repo: &str, reference: &str, body: &[u8]) -> u16 {
    send(
        port,
        "PUT",
        &format!("/v2/{repo}/manifests/{reference}"),
        &[("Content-Type", MANIFEST_TYPE)],
        body,
    )
    .status
}

// ---------------------------------------------------------------------------
// Rule 1: two pushes of the same blob
// "Both sessions are independent. Both finish 201. The store keeps one file.
//  Each repository gets its own link."
// ---------------------------------------------------------------------------

#[test]
fn same_blob_twice() {
    let server = start();
    let port = server.port;
    let bytes = Arc::new(blob("same_blob_twice", 2 << 20));
    let gate = Arc::new(Barrier::new(CLIENTS));

    let results: Vec<(u16, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..CLIENTS)
            .map(|index| {
                let bytes = Arc::clone(&bytes);
                let gate = Arc::clone(&gate);
                scope.spawn(move || {
                    let repo = format!("race/same-{index}");
                    gate.wait();
                    push_blob(port, &repo, &bytes)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread")).collect()
    });

    let digest = digest_of(&bytes);
    for (index, (status, seen)) in results.iter().enumerate() {
        assert_eq!(*status, 201, "session {index} did not finish 201");
        assert_eq!(seen, &digest, "session {index} produced another digest");
    }
    // Each repository has its own link, and every one of them serves the blob.
    for index in 0..CLIENTS {
        let got = send(
            port,
            "GET",
            &format!("/v2/race/same-{index}/blobs/{digest}"),
            &[],
            b"",
        );
        assert_eq!(got.status, 200, "repository {index} cannot read the blob");
        assert_eq!(got.body.len(), bytes.len(), "repository {index} short read");
    }
}

// ---------------------------------------------------------------------------
// Rule 2: delete racing a pull
// "A blob GET holds an open file handle. Delete removes the link only.
//  The pull completes."
// ---------------------------------------------------------------------------

#[test]
fn delete_during_pull() {
    let server = start();
    let port = server.port;
    let bytes = blob("delete_during_pull", 8 << 20);
    let (status, digest) = push_blob(port, "race/pull", &bytes);
    assert_eq!(status, 201);

    // Open the pull and read only the head of the body, so the response is in
    // flight while the delete lands.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("timeout");
    write!(
        stream,
        "GET /v2/race/pull/blobs/{digest} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("send");
    stream.flush().expect("flush");
    let mut head = vec![0_u8; 4096];
    let first = stream.read(&mut head).expect("first read");
    assert!(first > 0, "the pull sent nothing");

    let deleted = send(
        port,
        "DELETE",
        &format!("/v2/race/pull/blobs/{digest}"),
        &[],
        b"",
    );
    assert_eq!(deleted.status, 202, "the delete was refused");

    // The pull completes: the whole body arrives despite the link going away.
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).expect("finish the pull");
    let mut raw = head[..first].to_vec();
    raw.extend_from_slice(&rest);
    let answer = parse(&raw);
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.body.len(),
        bytes.len(),
        "the pull was cut short by the delete"
    );
    assert_eq!(answer.body, bytes, "the pull returned other bytes");

    // And the link really is gone for the next reader.
    let after = send(
        port,
        "GET",
        &format!("/v2/race/pull/blobs/{digest}"),
        &[],
        b"",
    );
    assert_eq!(after.status, 404, "the link outlived the delete");
}

// ---------------------------------------------------------------------------
// Rule 3: tag moved while it is read
// "A manifest GET by tag resolves tag to digest once, then serves that digest.
//  The response is always one consistent manifest, old or new."
// ---------------------------------------------------------------------------

#[test]
fn tag_moved_during_get() {
    let server = start();
    let port = server.port;
    let (status, layer) = push_blob(port, "race/tag", &blob("tag_moved", 1 << 20));
    assert_eq!(status, 201);

    let old = manifest_for(&layer, "old");
    let new = manifest_for(&layer, "new");
    let (old_digest, new_digest) = (digest_of(&old), digest_of(&new));
    assert_eq!(put_manifest(port, "race/tag", "live", &old), 201);

    let gate = Arc::new(Barrier::new(CLIENTS + 1));
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..CLIENTS)
            .map(|_| {
                let gate = Arc::clone(&gate);
                scope.spawn(move || {
                    gate.wait();
                    send(
                        port,
                        "GET",
                        "/v2/race/tag/manifests/live",
                        &[("Accept", MANIFEST_TYPE)],
                        b"",
                    )
                })
            })
            .collect();
        let writer = {
            let gate = Arc::clone(&gate);
            let new = new.clone();
            scope.spawn(move || {
                gate.wait();
                put_manifest(port, "race/tag", "live", &new)
            })
        };
        assert_eq!(writer.join().expect("writer"), 201);
        for reader in readers {
            let answer = reader.join().expect("reader");
            assert_eq!(answer.status, 200);
            let body_digest = digest_of(&answer.body);
            // One consistent manifest: old or new, never a mixture, and the
            // header always names the bytes that were actually sent.
            assert!(
                body_digest == old_digest || body_digest == new_digest,
                "a reader saw neither manifest whole"
            );
            assert_eq!(
                answer.header("docker-content-digest"),
                Some(body_digest.as_str()),
                "the digest header does not name the body"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Rule 4: two PATCH on one session at once
// "One proceeds; the other gets 416 RANGE_INVALID because its offset is stale.
//  A per-session mutex orders them."
// ---------------------------------------------------------------------------

#[test]
fn concurrent_patch_one_session() {
    let server = start();
    let port = server.port;
    let begin = send(port, "POST", "/v2/race/patch/blobs/uploads/", &[], b"");
    assert_eq!(begin.status, 202);
    let session = begin.location();

    // Both start at offset 0. Exactly one can be right.
    let chunk = blob("concurrent_patch", 1 << 20);
    let gate = Arc::new(Barrier::new(CLIENTS));
    let answers: Vec<Option<Answer>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..CLIENTS)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let session = session.clone();
                let chunk = chunk.clone();
                scope.spawn(move || {
                    gate.wait();
                    try_send(
                        port,
                        "PATCH",
                        &session,
                        &[
                            ("Content-Type", "application/octet-stream"),
                            ("Content-Range", &format!("0-{}", chunk.len() - 1)),
                        ],
                        &chunk,
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread")).collect()
    });

    let accepted = answers
        .iter()
        .filter(|a| a.as_ref().is_some_and(|a| a.status == 202))
        .count();
    assert_eq!(accepted, 1, "more than one PATCH at offset 0 was accepted");
    for answer in &answers {
        // A loser either reads its refusal, or finds the connection already
        // closed: the server may answer and hang up before the body is fully
        // sent, which is legal HTTP and is what a large chunk often sees. What
        // it must never do is succeed.
        let Some(answer) = answer else { continue };
        if answer.status == 202 {
            continue;
        }
        assert_eq!(answer.status, 416, "a stale PATCH was neither 202 nor 416");
        let body = String::from_utf8_lossy(&answer.body);
        assert!(
            body.contains("RANGE_INVALID"),
            "416 without RANGE_INVALID: {body}"
        );
    }

    // The session is still usable: the accepted chunk is the whole of it.
    let status = send(port, "GET", &session, &[], b"");
    assert_eq!(status.status, 204);
    assert_eq!(
        status.header("range"),
        Some(format!("0-{}", chunk.len() - 1).as_str()),
        "the session's range does not match the one accepted PATCH"
    );
}

// ---------------------------------------------------------------------------
// Rule 5: PUT manifest twice with the same tag
// "Last writer wins; each write is one tag_set. Both digests stay linked."
// ---------------------------------------------------------------------------

#[test]
fn tag_last_writer_wins() {
    let server = start();
    let port = server.port;
    let (status, layer) = push_blob(port, "race/last", &blob("last_writer", 1 << 20));
    assert_eq!(status, 201);

    let bodies: Vec<Vec<u8>> = (0..CLIENTS)
        .map(|index| manifest_for(&layer, &format!("writer-{index}")))
        .collect();
    let digests: Vec<String> = bodies.iter().map(|body| digest_of(body)).collect();

    let gate = Arc::new(Barrier::new(CLIENTS));
    std::thread::scope(|scope| {
        let handles: Vec<_> = bodies
            .iter()
            .map(|body| {
                let gate = Arc::clone(&gate);
                let body = body.clone();
                scope.spawn(move || {
                    gate.wait();
                    put_manifest(port, "race/last", "rolling", &body)
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().expect("writer"), 201);
        }
    });

    // The tag names exactly one of them, whole.
    let got = send(
        port,
        "GET",
        "/v2/race/last/manifests/rolling",
        &[("Accept", MANIFEST_TYPE)],
        b"",
    );
    assert_eq!(got.status, 200);
    let winner = digest_of(&got.body);
    assert!(digests.contains(&winner), "the tag names no manifest we wrote");

    // Every digest stays linked: a push by digest is not undone by losing the
    // race for the tag.
    for digest in &digests {
        let by_digest = send(
            port,
            "GET",
            &format!("/v2/race/last/manifests/{digest}"),
            &[("Accept", MANIFEST_TYPE)],
            b"",
        );
        assert_eq!(
            by_digest.status, 200,
            "{digest} lost its link by losing the tag"
        );
    }
}
