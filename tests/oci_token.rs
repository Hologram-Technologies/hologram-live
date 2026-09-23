#![cfg(feature = "oci")]
//! `auth.token`: the login every public registry uses, and the one shape that
//! gives anonymous pull and authenticated push at once.
//!
//! A password file protects reads as well as writes, so a registry that wants
//! public pulls cannot use one; and a registry that answers `/v2/` with 200
//! never teaches a client to log in, which is why skopeo and podman cannot
//! push to it. A bearer challenge solves both: anonymous clients fetch a
//! pull-only token, and a push fetches one with a password.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The gates' own password file: `gate` / `gate-password`.
const USER: &str = "gate";
const PASSWORD: &str = "gate-password";
const SERVICE: &str = "hologram-registry";

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
    let slash = |path: &std::path::Path| path.display().to_string().replace('\\', "/");
    let file = root.path().join("config.yml");
    std::fs::write(
        &file,
        format!(
            "version: 0.1\nlog:\n  level: warn\nstorage:\n  filesystem:\n    rootdirectory: {}\nhttp:\n  addr: 127.0.0.1:{port}\nauth:\n  token:\n    realm: http://127.0.0.1:{port}/auth/token\n    service: {SERVICE}\n    issuer: {SERVICE}\n    local: true\n  htpasswd:\n    realm: Registry Realm\n    path: {}\n",
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

/// One request, with an optional `Authorization` header: status, headers, body.
fn send(
    port: u16,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
) -> (u16, String, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let auth = authorization.map_or(String::new(), |value| format!("Authorization: {value}\r\n"));
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).expect("send");
    stream.write_all(body).expect("body");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status");
    (status, head, raw[split + 4..].to_vec())
}

fn basic(user: &str, password: &str) -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

/// Fetch a token the way a client does after reading the challenge.
fn token(port: u16, scope: &str, login: Option<(&str, &str)>) -> String {
    let path = format!(
        "/auth/token?service={SERVICE}&scope={}",
        scope.replace(':', "%3A")
    );
    let authorization = login.map(|(user, password)| basic(user, password));
    let (status, _, body) = send(port, "GET", &path, authorization.as_deref(), b"");
    assert_eq!(
        status,
        200,
        "the realm issued no token: {}",
        String::from_utf8_lossy(&body)
    );
    let value: serde_json::Value = serde_json::from_slice(&body).expect("a token answer");
    value["token"].as_str().expect("a token").to_owned()
}

/// The whole flow a client walks: a challenge that names the realm, a token
/// from it, and what each token may do.
#[test]
fn a_challenge_sends_the_client_for_a_token_that_says_what_it_may_do() {
    let server = start();
    let port = server.port;

    // 1. The base route challenges, and names where to go.
    let (status, head, _) = send(port, "GET", "/v2/", None, b"");
    assert_eq!(status, 401);
    let challenge = head
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("www-authenticate:"))
        .expect("a challenge");
    assert!(challenge.contains("Bearer realm="), "{challenge}");
    assert!(challenge.contains("/auth/token"), "{challenge}");
    assert!(
        challenge.contains(&format!("service=\"{SERVICE}\"")),
        "{challenge}"
    );

    // 2. Anonymous gets a token, and it opens the base route.
    let anonymous = token(port, "repository:team/app:pull", None);
    let (status, _, _) = send(
        port,
        "GET",
        "/v2/",
        Some(&format!("Bearer {anonymous}")),
        b"",
    );
    assert_eq!(status, 200, "an anonymous token opens the base route");

    // 3. Anonymous may not push: the registry challenges again, naming the
    //    scope it wants.
    let (status, head, _) = send(
        port,
        "POST",
        "/v2/team/app/blobs/uploads/",
        Some(&format!("Bearer {anonymous}")),
        b"",
    );
    assert_eq!(status, 401, "anonymous pushed");
    assert!(
        head.contains("repository:team/app:pull,push"),
        "the challenge names the scope: {head}"
    );

    // 4. A token asked for with the password carries push.
    let pusher = token(
        port,
        "repository:team/app:pull,push",
        Some((USER, PASSWORD)),
    );
    let (status, head, _) = send(
        port,
        "POST",
        "/v2/team/app/blobs/uploads/",
        Some(&format!("Bearer {pusher}")),
        b"",
    );
    assert_eq!(status, 202, "the push was refused: {head}");

    // 5. A wrong password gets no token at all.
    let (status, _, _) = send(
        port,
        "GET",
        &format!("/auth/token?service={SERVICE}&scope=repository%3Ateam%2Fapp%3Apull%2Cpush"),
        Some(&basic(USER, "not-the-password")),
        b"",
    );
    assert_eq!(status, 401);

    // 6. The key that signed all this is published, so anything else can check.
    let (status, head, body) = send(port, "GET", "/auth/jwks.json", None, b"");
    assert_eq!(status, 200);
    assert!(head.contains("application/jwk-set+json"), "{head}");
    let keys: serde_json::Value = serde_json::from_slice(&body).expect("a key set");
    assert_eq!(keys["keys"][0]["crv"], "P-256");
    assert!(keys["keys"][0]["x"].is_string());
}

/// A token for one repository does not open another, and a token for pull
/// does not push: the scope is the whole point.
#[test]
fn a_token_opens_only_what_it_names() {
    let server = start();
    let port = server.port;
    let mine = token(
        port,
        "repository:team/mine:pull,push",
        Some((USER, PASSWORD)),
    );

    let (status, _, _) = send(
        port,
        "POST",
        "/v2/team/mine/blobs/uploads/",
        Some(&format!("Bearer {mine}")),
        b"",
    );
    assert_eq!(status, 202, "its own repository");

    let (status, _, _) = send(
        port,
        "POST",
        "/v2/team/other/blobs/uploads/",
        Some(&format!("Bearer {mine}")),
        b"",
    );
    assert_eq!(status, 401, "someone else's repository");

    let (status, _, _) = send(
        port,
        "GET",
        "/v2/team/mine/tags/list",
        Some(&format!("Bearer {mine}")),
        b"",
    );
    assert_eq!(
        status, 404,
        "a read it may do: the repository is simply empty"
    );

    // A token nobody signed is no token.
    let (status, _, _) = send(
        port,
        "GET",
        "/v2/team/mine/tags/list",
        Some("Bearer not.a.token"),
        b"",
    );
    assert_eq!(status, 401);
}
