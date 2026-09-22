#![cfg(feature = "oci")]
//! The debug listener (plan P6 T5, FR-S09), through the real binary started
//! from a `config.yml`, as the image's default file asks for it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    public: u16,
    debug: u16,
    _root: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
        .port()
}

fn start() -> Server {
    let root = tempfile::tempdir().expect("tempdir");
    let (public, debug) = (free_port(), free_port());
    let file = root.path().join("config.yml");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{public}\n  debug:\n    addr: 127.0.0.1:{debug}\n    prometheus:\n      enabled: true\n      path: /metrics\nhealth:\n  storagedriver:\n    enabled: true\n    interval: 1s\n    threshold: 3\n",
            root.path().join("volume").display().to_string().replace('\\', "/")
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
        public,
        debug,
        _root: root,
    };
    let deadline = Instant::now() + Duration::from_mins(1);
    while TcpStream::connect(("127.0.0.1", public)).is_err() {
        assert!(Instant::now() < deadline, "the registry did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    server
}

/// One HTTP/1.1 exchange: the status and the body.
fn request(port: u16, method: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").expect("send");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a head");
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status");
    (status, body.to_owned())
}

#[test]
fn health_answers_as_the_reference_and_the_manual_drain_flips_it() {
    let server = start();
    assert_eq!(
        request(server.debug, "GET", "/debug/health"),
        (200, "{}".to_owned())
    );
    assert_eq!(request(server.debug, "POST", "/debug/health/down").0, 200);
    let (status, body) = request(server.debug, "GET", "/debug/health");
    assert_eq!(status, 503);
    assert_eq!(body, "{\"manual_http_status\":\"Manual Check\"}");
    // The drain: the public port answers 503 UNAVAILABLE while a check fails.
    let (status, body) = request(server.public, "GET", "/v2/");
    assert_eq!(status, 503);
    assert!(
        body.contains("UNAVAILABLE") && body.contains("please see /debug/health"),
        "{body}"
    );
    assert_eq!(request(server.debug, "POST", "/debug/health/up").0, 200);
    assert_eq!(request(server.debug, "GET", "/debug/health").0, 200);
    assert_eq!(request(server.public, "GET", "/v2/").0, 200, "undrained");
    // The storage check runs every second and passes on a writable volume.
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        request(server.debug, "GET", "/debug/health"),
        (200, "{}".to_owned())
    );
    // Go's internals are not served.
    assert_eq!(request(server.debug, "GET", "/debug/vars").0, 404);
    assert_eq!(request(server.debug, "GET", "/debug/pprof/").0, 404);
}

#[test]
fn metrics_count_requests_by_route_never_by_repository() {
    let server = start();
    assert_eq!(request(server.public, "GET", "/v2/").0, 200);
    assert_eq!(
        request(server.public, "GET", "/v2/team/secret-name/tags/list").0,
        404
    );
    let (status, text) = request(server.debug, "GET", "/metrics");
    assert_eq!(status, 200);
    assert!(
        text.contains(
            "registry_http_requests_total{code=\"200\",handler=\"base\",method=\"get\"} 1"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "registry_http_requests_total{code=\"404\",handler=\"tags\",method=\"get\"} 1"
        ),
        "{text}"
    );
    assert!(
        text.contains("# TYPE registry_http_request_duration_seconds histogram"),
        "{text}"
    );
    assert!(text.contains("hologram_build_info{"), "{text}");
    assert!(
        !text.contains("secret-name"),
        "a repository name became a label value"
    );
    // The public port does not serve the debug routes.
    assert_eq!(request(server.public, "GET", "/metrics").0, 404);
    assert_eq!(request(server.public, "GET", "/debug/health").0, 404);
}
