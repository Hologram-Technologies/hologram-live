#![cfg(all(feature = "oci", target_os = "linux"))]
//! SC-007 through the router: memory must not grow with the size of a layer.
//!
//! `tests/oci_store.rs::two_gib_stay_under_200_mb` proves it of the store. This
//! proves it of the thing an operator actually runs: 2 GiB pushed and pulled
//! over HTTP, with the server's own resident memory sampled while it happens.
//! The static counterpart is `scripts/check-oci-streaming.sh`, which bans
//! whole-blob reads in `src/oci_store/`; this is the dynamic one.
//!
//! Ignored by default: it moves 2 GiB twice. `gates-nightly.yml` runs it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 4 MiB, the frame the store writes in.
const CHUNK: usize = 4 << 20;
/// 512 chunks: 2 GiB.
const CHUNKS: usize = 512;
/// The same ceiling the store-level test holds.
const CEILING_MB: u64 = 200;

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

fn start() -> Server {
    let root = tempfile::tempdir().expect("tempdir");
    let port = free_port();
    let file = root.path().join("config.yml");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{port}\n",
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

/// The server's resident set, in bytes. Its own, not ours: the whole point is
/// what the process an operator runs does with a 2 GiB layer.
fn rss_bytes(pid: u32) -> u64 {
    let statm = match std::fs::read_to_string(format!("/proc/{pid}/statm")) {
        Ok(text) => text,
        // The process is gone; the last sample stands.
        Err(_) => return 0,
    };
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|pages| pages.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096)
}

/// Chunk `index`, never repeating, so a misplaced offset changes the digest.
fn chunk(index: usize) -> Vec<u8> {
    let mut out = vec![0_u8; CHUNK];
    blake3::Hasher::new()
        .update(&(index as u64).to_le_bytes())
        .finalize_xof()
        .fill(&mut out);
    out
}

fn read_answer(stream: &mut TcpStream) -> (u16, Vec<(String, String)>) {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte).expect("read head");
        assert!(read == 1, "the connection closed inside the head");
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status");
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect();
    (status, headers)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn path_of(location: &str) -> String {
    match location.find("/v2/") {
        Some(at) => location[at..].to_owned(),
        None => location.to_owned(),
    }
}

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .expect("timeout");
    stream
}

#[test]
#[ignore = "moves 2 GiB twice; gates-nightly.yml runs it"]
fn two_gib_through_the_router_stays_under_the_ceiling() {
    let server = start();
    let (port, pid) = (server.port, server.child.id());

    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(0));
    let sampler = {
        let (stop, peak) = (Arc::clone(&stop), Arc::clone(&peak));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(rss_bytes(pid), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };

    // Begin the session.
    let mut stream = connect(port);
    write!(
        stream,
        "POST /v2/big/layer/blobs/uploads/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
    .expect("POST");
    stream.flush().expect("flush");
    let (status, headers) = read_answer(&mut stream);
    assert_eq!(status, 202, "POST upload");
    let session = path_of(header(&headers, "location").expect("a Location"));
    drop(stream);

    // One PATCH whose body is 2 GiB, written 4 MiB at a time and never held
    // whole on either side. The digest is computed as the bytes go past.
    let total = CHUNK * CHUNKS;
    let mut stream = connect(port);
    write!(
        stream,
        "PATCH {session} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {total}\r\n\r\n"
    )
    .expect("PATCH head");
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    for index in 0..CHUNKS {
        let bytes = chunk(index);
        sha2::Digest::update(&mut hasher, &bytes);
        stream.write_all(&bytes).expect("PATCH body");
    }
    stream.flush().expect("flush");
    let digest = format!("sha256:{:x}", sha2::Digest::finalize(hasher));
    let (status, headers) = read_answer(&mut stream);
    assert_eq!(status, 202, "PATCH 2 GiB");
    let session = path_of(header(&headers, "location").unwrap_or(&session));
    drop(stream);

    // Finish it.
    let mut stream = connect(port);
    write!(
        stream,
        "PUT {session}?digest={digest} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
    .expect("PUT");
    stream.flush().expect("flush");
    let (status, _) = read_answer(&mut stream);
    assert_eq!(status, 201, "PUT upload");
    drop(stream);

    // Pull it back, discarding as it arrives, and count what came.
    let mut stream = connect(port);
    write!(
        stream,
        "GET /v2/big/layer/blobs/{digest} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("GET");
    stream.flush().expect("flush");
    let (status, headers) = read_answer(&mut stream);
    assert_eq!(status, 200, "GET blob");
    assert_eq!(
        header(&headers, "content-length"),
        Some(total.to_string().as_str()),
        "the pull does not offer the whole layer"
    );
    let mut seen = 0_u64;
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => seen += read as u64,
            Err(error) => panic!("pull failed after {seen} bytes: {error}"),
        }
    }
    assert_eq!(seen, total as u64, "the pull was short");

    stop.store(true, Ordering::Relaxed);
    sampler.join().expect("sampler");
    let peak_mb = peak.load(Ordering::Relaxed) / (1024 * 1024);
    println!("SC-007 through the router: peak server RSS {peak_mb} MB moving 2 GiB in and out");
    assert!(
        peak_mb < CEILING_MB,
        "peak RSS was {peak_mb} MB, over the {CEILING_MB} MB ceiling"
    );
}
