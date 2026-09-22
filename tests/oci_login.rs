#![cfg(feature = "oci")]
//! Login from a password file (`auth.htpasswd`), through the registry's
//! handler, measured against the reference's code (distribution v3.1.1,
//! `registry/auth/htpasswd` and `registry/handlers/app.go`).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use hologram_live::modules::oci::auth::Htpasswd;
use hologram_live::modules::oci::{handle, Registry, Settings};
use hologram_live::oci_store::{OciStore, OpenOptions};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const REALM: &str = "Registry Realm";

struct Fixture {
    _dir: tempfile::TempDir,
    file: std::path::PathBuf,
    registry: Registry,
}

/// bcrypt at the lowest cost: the tests check the rule, not the cost.
fn entry(user: &str, password: &str) -> String {
    let hash = bcrypt::hash(password, 4).expect("hash");
    format!("{user}:{}", hash.replacen("$2b$", "$2y$", 1))
}

fn fixture(lines: &[String]) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("auth/htpasswd");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("auth directory");
    std::fs::write(&file, lines.join("\n")).expect("htpasswd");
    let store = OciStore::open(
        &dir.path().join("volume"),
        OpenOptions {
            create: true,
            upload_max_age: Duration::from_hours(1),
        },
    )
    .expect("volume");
    let login = Htpasswd::open(file.clone(), REALM.to_owned()).expect("login");
    let registry = Registry {
        store: Arc::new(store),
        settings: Settings::default(),
        audit: None,
        login: Some(Arc::new(login)),
    };
    Fixture {
        _dir: dir,
        file,
        registry,
    }
}

fn basic(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

async fn send(
    fixture: &Fixture,
    method: &str,
    path: &str,
    authorization: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    handle(
        fixture.registry.clone(),
        request.body(Body::empty()).expect("request"),
    )
    .await
}

async fn body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1 << 20).await.expect("body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn detail(records: &[(&str, &str, &str)]) -> Value {
    Value::Array(
        records
            .iter()
            .map(|(kind, name, action)| json!({ "Type": kind, "Class": "", "Name": name, "Action": action }))
            .collect(),
    )
}

#[tokio::test]
async fn no_credentials_is_the_references_basic_challenge_naming_what_was_asked() {
    let fixture = fixture(&[entry("gate", "secret")]);
    let cases = [
        ("GET", "/v2/team/app/manifests/v1", detail(&[("repository", "team/app", "pull")])),
        ("HEAD", "/v2/team/app/blobs/sha256:0000000000000000000000000000000000000000000000000000000000000000", detail(&[("repository", "team/app", "pull")])),
        ("POST", "/v2/team/app/blobs/uploads/", detail(&[("repository", "team/app", "pull"), ("repository", "team/app", "push")])),
        (
            "POST",
            "/v2/team/app/blobs/uploads/?mount=sha256:0000000000000000000000000000000000000000000000000000000000000000&from=team%2Fbase",
            detail(&[("repository", "team/app", "pull"), ("repository", "team/app", "push"), ("repository", "team/base", "pull")]),
        ),
        ("DELETE", "/v2/team/app/manifests/sha256:0000000000000000000000000000000000000000000000000000000000000000", detail(&[("repository", "team/app", "delete")])),
        ("GET", "/v2/_catalog", detail(&[("registry", "catalog", "*")])),
    ];
    for (method, path, expected) in cases {
        let response = send(&fixture, method, path, None).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        assert_eq!(
            response.headers()["www-authenticate"],
            "Basic realm=\"Registry Realm\"",
            "{method} {path}"
        );
        let answer = body(response).await;
        assert_eq!(
            answer["errors"][0]["code"], "UNAUTHORIZED",
            "{method} {path}"
        );
        assert_eq!(answer["errors"][0]["message"], "authentication required");
        assert_eq!(answer["errors"][0]["detail"], expected, "{method} {path}");
    }
    // The base route is challenged too: it is what `docker login` probes.
    let base = send(&fixture, "GET", "/v2/", None).await;
    assert_eq!(base.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_good_login_passes_and_a_bad_one_is_challenged_the_same_way_for_any_reason() {
    let fixture = fixture(&[
        "# a comment".to_owned(),
        String::new(),
        entry("gate", "secret"),
        // `htpasswd -m`: not bcrypt, so it never logs in, as in the reference.
        "md5user:$apr1$r31.....$HqJZimcKQFAMYayBlzkrA/".to_owned(),
    ]);
    let good = send(&fixture, "GET", "/v2/", Some(&basic("gate", "secret"))).await;
    assert_eq!(good.status(), StatusCode::OK);
    for (user, password) in [
        ("gate", "wrong"),
        ("nobody", "secret"),
        ("md5user", "secret"),
    ] {
        let refused = send(&fixture, "GET", "/v2/", Some(&basic(user, password))).await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED, "{user}");
        assert!(refused.headers().contains_key("www-authenticate"));
    }
    // Not Basic, and not base64: challenged, not an error.
    for header in ["Bearer abc", "Basic !!!", "basic"] {
        let refused = send(&fixture, "GET", "/v2/", Some(header)).await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED, "{header}");
    }
    // The scheme is case-insensitive, as Go reads it.
    let lower = basic("gate", "secret").replacen("Basic", "basic", 1);
    assert_eq!(
        send(&fixture, "GET", "/v2/", Some(&lower)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_changed_file_is_read_again_and_a_remembered_login_goes_with_the_user() {
    let fixture = fixture(&[entry("gate", "secret")]);
    let ok = |response: &Response| response.status() == StatusCode::OK;
    assert!(ok(&send(
        &fixture,
        "GET",
        "/v2/",
        Some(&basic("gate", "secret"))
    )
    .await));
    // Replace the user; the modification time must move for the file to be read.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&fixture.file, entry("other", "new")).expect("rewrite");
    assert!(
        !ok(&send(&fixture, "GET", "/v2/", Some(&basic("gate", "secret"))).await),
        "a removed user is not remembered"
    );
    assert!(ok(&send(
        &fixture,
        "GET",
        "/v2/",
        Some(&basic("other", "new"))
    )
    .await));
}

#[tokio::test]
async fn a_file_that_cannot_be_read_is_a_bare_400_as_the_reference() {
    let fixture = fixture(&["no colon on this line".to_owned()]);
    let response = send(&fixture, "GET", "/v2/", Some(&basic("gate", "secret"))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 1024).await.expect("body");
    assert!(bytes.is_empty(), "no information, as the reference");
}

#[test]
fn a_missing_file_is_created_with_one_user_as_the_reference() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("new/auth/htpasswd");
    Htpasswd::open(file.clone(), REALM.to_owned()).expect("created");
    let text = std::fs::read_to_string(&file).expect("the file");
    let (user, hash) = text.split_once(':').expect("user:hash");
    assert_eq!(user, "docker");
    assert!(hash.starts_with("$2") && !text.ends_with('\n'), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).expect("file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // An existing file is left as it is.
    std::fs::write(&file, "kept").expect("write");
    Htpasswd::open(file.clone(), REALM.to_owned()).expect("open");
    assert_eq!(std::fs::read_to_string(&file).expect("read"), "kept");
}

/// The reference's router matches by path; its dispatcher authorizes before
/// the handler checks the method, the upload id or the digest. So these are
/// challenged, not answered with 405 or an upload error.
#[tokio::test]
async fn what_the_references_router_matches_is_challenged_before_anything_else() {
    let fixture = fixture(&[entry("gate", "secret")]);
    let cases = [
        (
            "PATCH",
            "/v2/team/app/manifests/v1",
            detail(&[
                ("repository", "team/app", "pull"),
                ("repository", "team/app", "push"),
            ]),
        ),
        (
            "GET",
            "/v2/team/app/blobs/uploads/",
            detail(&[("repository", "team/app", "pull")]),
        ),
        (
            "GET",
            "/v2/team/app/blobs/uploads/not-an-id",
            detail(&[("repository", "team/app", "pull")]),
        ),
        (
            "GET",
            "/v2/team/app/blobs/sha256:abc",
            detail(&[("repository", "team/app", "pull")]),
        ),
        (
            "POST",
            "/v2/_catalog",
            detail(&[("registry", "catalog", "*")]),
        ),
        (
            "OPTIONS",
            "/v2/_catalog",
            detail(&[("registry", "catalog", "*")]),
        ),
        (
            "OPTIONS",
            "/v2/team/app/blobs/uploads/?from=team%2Fbase",
            detail(&[("repository", "team/base", "pull")]),
        ),
    ];
    for (method, path, expected) in cases {
        let response = send(&fixture, method, path, None).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        assert_eq!(
            body(response).await["errors"][0]["detail"],
            expected,
            "{method} {path}"
        );
    }
    // The base route and a bare OPTIONS name nothing: Go sends `"detail": null`.
    for (method, path) in [("GET", "/v2/"), ("OPTIONS", "/v2/team/app/manifests/v1")] {
        let answer = body(send(&fixture, method, path, None).await).await;
        let error = answer["errors"][0].as_object().expect("an error").clone();
        assert_eq!(error.get("detail"), Some(&Value::Null), "{method} {path}");
    }
    // A path the reference's router does not match is its plain 404, login or not.
    let none = send(&fixture, "GET", "/v2/team/app/nothing/here", None).await;
    assert_eq!(none.status(), StatusCode::NOT_FOUND);
}

#[test]
fn a_realm_outside_ascii_is_still_sent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("htpasswd");
    std::fs::write(&file, entry("gate", "secret")).expect("write");
    let login = Arc::new(Htpasswd::open(file, "Réalm \"q\"".to_owned()).expect("open"));
    let registry = Registry {
        store: Arc::new(
            OciStore::open(
                &dir.path().join("volume"),
                OpenOptions {
                    create: true,
                    upload_max_age: Duration::from_hours(1),
                },
            )
            .expect("volume"),
        ),
        settings: Settings::default(),
        audit: None,
        login: Some(login),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let response = runtime.block_on(handle(
        registry,
        Request::builder()
            .uri("/v2/")
            .body(Body::empty())
            .expect("request"),
    ));
    let challenge = response
        .headers()
        .get("www-authenticate")
        .expect("a challenge");
    assert_eq!(
        challenge.as_bytes(),
        "Basic realm=\"Réalm \\\"q\\\"\"".as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn provisioning_leaves_a_directory_that_is_already_there_as_it_is() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let shared = dir.path().join("shared");
    std::fs::create_dir(&shared).expect("shared");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).expect("mode");
    Htpasswd::open(shared.join("new/htpasswd"), REALM.to_owned()).expect("provisioned");
    let mode = |path: &std::path::Path| {
        std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
    };
    assert_eq!(
        mode(&shared),
        0o755,
        "an existing directory is not made 0700"
    );
    assert_eq!(mode(&shared.join("new")), 0o700, "a new one is 0700");
}
