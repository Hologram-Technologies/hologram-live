#![cfg(feature = "oci")]
//! The registry's read path over HTTP: first against the handler, then
//! through the real binary, where the bearer layer and the HTTP stack are.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use hologram_live::modules::oci::{handle, Registry, Settings};
use hologram_live::oci_store::{
    Digest, LinkKind, ManifestPlan, OciStore, OpenOptions, Reference, RepoName,
};
use std::sync::Arc;
use std::time::Duration;

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

fn options() -> OpenOptions {
    OpenOptions {
        create: true,
        upload_max_age: Duration::from_hours(7 * 24),
    }
}

fn repo(name: &str) -> RepoName {
    RepoName::parse(name).expect("repository name")
}

/// 3 MiB that do not repeat, so a range that starts in the wrong place shows.
fn layer() -> Vec<u8> {
    let mut out = vec![0_u8; 3 << 20];
    blake3::Hasher::new()
        .update(b"oci_http layer")
        .finalize_xof()
        .fill(&mut out);
    out
}

fn push_blob(store: &OciStore, name: &str, bytes: &[u8]) -> Digest {
    let id = store.upload_begin(&repo(name)).expect("begin");
    store.upload_append(&id, 0, bytes).expect("append");
    let digest = Digest::sha256_of(bytes);
    store.upload_finish(&id, &digest).expect("finish")
}

/// One layer and one manifest tagged `v1` in `team/app`. Returns the layer's
/// digest, the manifest's digest and the manifest's bytes.
fn seed(store: &OciStore) -> (Digest, Digest, Vec<u8>) {
    let layer_digest = push_blob(store, "team/app", &layer());
    let manifest = format!(
        r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_TYPE}","layers":[{{"digest":"{layer_digest}"}}]}}"#
    )
    .into_bytes();
    let plan = ManifestPlan {
        kind: LinkKind::Manifest,
        must_exist: vec![layer_digest.clone()],
        subject: None,
    };
    let digest = store
        .manifest_put(
            &repo("team/app"),
            &Reference::parse("v1").expect("tag"),
            MANIFEST_TYPE,
            &manifest,
            &plan,
        )
        .expect("manifest");
    (layer_digest, digest, manifest)
}

struct Volume {
    _dir: tempfile::TempDir,
    store: Arc<OciStore>,
    /// `storage.delete.enabled`. Off, as in the reference, unless a test turns it on.
    delete_enabled: bool,
    /// `http.headers`.
    headers: Vec<(&'static str, &'static str)>,
}

fn volume() -> Volume {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(OciStore::open(dir.path(), options()).expect("open"));
    Volume {
        _dir: dir,
        store,
        delete_enabled: false,
        headers: Vec::new(),
    }
}

async fn send(volume: &Volume, method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
    send_body(volume, method, path, headers, Vec::new()).await
}

async fn send_body(
    volume: &Volume,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    // The reference serves an OCI manifest only to a client that asks for the
    // type, so the tests ask, as every real client does.
    let asks = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("accept"));
    if path.contains("/manifests/") && !asks {
        request = request.header("accept", MANIFEST_TYPE);
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let registry = Registry {
        store: volume.store.clone(),
        settings: Settings {
            delete_enabled: volume.delete_enabled,
            headers: volume
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        axum::http::HeaderName::from_static(name),
                        axum::http::HeaderValue::from_static(value),
                    )
                })
                .collect(),
        },
        audit: None,
    };
    handle(registry, request.body(Body::from(body)).expect("request")).await
}

fn header<'a>(response: &'a Response, name: &str) -> &'a str {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("no {name} header"))
        .to_str()
        .expect("header text")
}

async fn body(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 8 << 20)
        .await
        .expect("body")
        .to_vec()
}

async fn error_code(response: Response) -> String {
    assert_eq!(header(&response, "content-type"), "application/json");
    assert_eq!(
        header(&response, "docker-distribution-api-version"),
        "registry/2.0"
    );
    let value: serde_json::Value = serde_json::from_slice(&body(response).await).expect("json");
    value["errors"][0]["code"]
        .as_str()
        .expect("error code")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_base_route_answers_an_empty_object() {
    let volume = volume();
    let response = send(&volume, "GET", "/v2/", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "application/json");
    assert_eq!(body(response).await, b"{}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_is_served_whole_with_its_headers() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let response = send(&volume, "GET", &format!("/v2/team/app/blobs/{digest}"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-length"), (3 << 20).to_string());
    assert_eq!(header(&response, "docker-content-digest"), digest.as_str());
    assert_eq!(header(&response, "etag"), format!("\"{digest}\""));
    assert_eq!(
        header(&response, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(header(&response, "accept-ranges"), "bytes");
    assert_eq!(body(response).await, layer());
}

#[tokio::test(flavor = "multi_thread")]
async fn head_says_the_length_and_sends_nothing() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let response = send(
        &volume,
        "HEAD",
        &format!("/v2/team/app/blobs/{digest}"),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-length"), (3 << 20).to_string());
    assert_eq!(header(&response, "docker-content-digest"), digest.as_str());
    assert!(body(response).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_range_forms_over_http() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let path = format!("/v2/team/app/blobs/{digest}");
    let bytes = layer();
    let size = bytes.len();

    let one = send(&volume, "GET", &path, &[("range", "bytes=1048570-1048589")]).await;
    assert_eq!(one.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header(&one, "content-range"),
        format!("bytes 1048570-1048589/{size}")
    );
    assert_eq!(header(&one, "content-length"), "20");
    assert_eq!(body(one).await, &bytes[1_048_570..1_048_590]);

    let open_ended = send(&volume, "GET", &path, &[("range", "bytes=3145700-")]).await;
    assert_eq!(open_ended.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(body(open_ended).await, &bytes[3_145_700..]);

    let suffix = send(&volume, "GET", &path, &[("range", "bytes=-16")]).await;
    assert_eq!(suffix.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(body(suffix).await, &bytes[size - 16..]);

    let past = send(&volume, "GET", &path, &[("range", "bytes=9999999-")]).await;
    assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&past, "content-range"), format!("bytes */{size}"));
    assert_eq!(body(past).await, b"invalid range: failed to overlap\n");
    let malformed = send(&volume, "GET", &path, &[("range", "bytes=9-3")]).await;
    assert_eq!(malformed.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(malformed.headers().get("content-range").is_none());
    assert_eq!(body(malformed).await, b"invalid range\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_is_served_only_from_a_repository_that_links_it() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let elsewhere = send(
        &volume,
        "GET",
        &format!("/v2/other/app/blobs/{digest}"),
        &[],
    )
    .await;
    assert_eq!(elsewhere.status(), StatusCode::NOT_FOUND);
    assert_eq!(error_code(elsewhere).await, "BLOB_UNKNOWN");

    let absent = Digest::sha256_of(b"never pushed");
    let missing = send(
        &volume,
        "HEAD",
        &format!("/v2/team/app/blobs/{absent}"),
        &[],
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let malformed = send(&volume, "GET", "/v2/team/app/blobs/sha256:abc", &[]).await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(malformed).await, "DIGEST_INVALID");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_answers_to_its_blake3_name_too() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let blake3 = Digest::from_blake3(&blake3::hash(&layer()));
    let response = send(&volume, "GET", &format!("/v2/team/app/blobs/{blake3}"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "docker-content-digest"),
        blake3.as_str(),
        "the client verifies against the name it asked by"
    );
    assert_ne!(blake3, digest);
    assert_eq!(body(response).await, layer());
}

#[tokio::test(flavor = "multi_thread")]
async fn if_none_match_is_304() {
    let volume = volume();
    let (digest, manifest_digest, _) = seed(&volume.store);
    let etag = format!("\"{digest}\"");
    let blob = send(
        &volume,
        "GET",
        &format!("/v2/team/app/blobs/{digest}"),
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(blob.status(), StatusCode::NOT_MODIFIED);
    assert!(body(blob).await.is_empty());

    let etag = format!("\"{manifest_digest}\"");
    let manifest = send(
        &volume,
        "GET",
        "/v2/team/app/manifests/v1",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(manifest.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_by_tag_and_by_digest() {
    let volume = volume();
    let (_, digest, bytes) = seed(&volume.store);
    for reference in ["v1".to_owned(), digest.to_string()] {
        let path = format!("/v2/team/app/manifests/{reference}");
        let response = send(&volume, "GET", &path, &[]).await;
        assert_eq!(response.status(), StatusCode::OK, "{reference}");
        assert_eq!(header(&response, "content-type"), MANIFEST_TYPE);
        assert_eq!(header(&response, "docker-content-digest"), digest.as_str());
        assert_eq!(header(&response, "etag"), format!("\"{digest}\""));
        assert_eq!(header(&response, "content-length"), bytes.len().to_string());
        assert_eq!(body(response).await, bytes);

        let head = send(&volume, "HEAD", &path, &[]).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(header(&head, "content-length"), bytes.len().to_string());
        assert!(body(head).await.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oci_manifest_is_served_only_to_a_client_that_asks_for_the_type() {
    let volume = volume();
    seed(&volume.store);
    for accept in [
        "*/*",
        "application/vnd.docker.distribution.manifest.v2+json",
    ] {
        let response = send(
            &volume,
            "GET",
            "/v2/team/app/manifests/v1",
            &[("accept", accept)],
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{accept}");
        let value: serde_json::Value = serde_json::from_slice(&body(response).await).expect("json");
        assert_eq!(value["errors"][0]["code"], "MANIFEST_UNKNOWN");
        assert_eq!(
            value["errors"][0]["message"],
            "OCI manifest found, but accept header does not support OCI manifests"
        );
    }
    let listed = format!("application/json, {MANIFEST_TYPE};q=0.9");
    let response = send(
        &volume,
        "GET",
        "/v2/team/app/manifests/v1",
        &[("accept", &listed)],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// Review of #92: a layer asked for through the manifest route is unknown,
/// and is never read whole to find that out.
#[tokio::test(flavor = "multi_thread")]
async fn a_layer_is_not_a_manifest() {
    let volume = volume();
    let (layer, _, _) = seed(&volume.store);
    let response = send(
        &volume,
        "GET",
        &format!("/v2/team/app/manifests/{layer}"),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(error_code(response).await, "MANIFEST_UNKNOWN");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_manifest_is_manifest_unknown() {
    let volume = volume();
    seed(&volume.store);
    for path in [
        "/v2/team/app/manifests/nope",
        "/v2/never/seen/manifests/v1",
        &format!("/v2/team/app/manifests/{}", Digest::sha256_of(b"absent")),
    ] {
        let response = send(&volume, "GET", path, &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(error_code(response).await, "MANIFEST_UNKNOWN", "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_percent_encoded_digest_is_read_once() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);
    let encoded = digest.as_str().replace(':', "%3A");
    let response = send(
        &volume,
        "HEAD",
        &format!("/v2/team/app/blobs/{encoded}"),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let hidden = send(&volume, "GET", "/v2/team%2Fapp/tags/list", &[]).await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_method_and_an_unknown_path() {
    let volume = volume();
    let wrong = send(&volume, "POST", "/v2/team/app/manifests/v1", &[]).await;
    assert_eq!(wrong.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header(&wrong, "allow"), "DELETE, GET, HEAD, PUT");
    let unknown = send(&volume, "GET", "/v2/team/app/nothing/here", &[]).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        header(&unknown, "docker-distribution-api-version"),
        "registry/2.0"
    );
}

// ---- The write path ----------------------------------------------------------

/// `POST`, then the location it answered with.
async fn open_upload(volume: &Volume, repo: &str) -> String {
    let response = send(volume, "POST", &format!("/v2/{repo}/blobs/uploads/"), &[]).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(header(&response, "range"), "0-0");
    assert_eq!(header(&response, "content-length"), "0");
    let location = header(&response, "location").to_owned();
    assert_eq!(
        location,
        format!(
            "/v2/{repo}/blobs/uploads/{}",
            header(&response, "docker-upload-uuid")
        )
    );
    location
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_pushed_in_chunks_reads_back() {
    let volume = volume();
    let bytes = layer();
    let digest = Digest::sha256_of(&bytes);
    let location = open_upload(&volume, "team/app").await;
    let (first, second) = bytes.split_at(1 << 20);

    // Without Content-Range, as docker sends it.
    let patched = send_body(&volume, "PATCH", &location, &[], first.to_vec()).await;
    assert_eq!(patched.status(), StatusCode::ACCEPTED);
    assert_eq!(header(&patched, "range"), "0-1048575");
    assert_eq!(header(&patched, "location"), location);

    // A chunk that does not start where the upload stands is refused, and told where it stands.
    let stale = send_body(
        &volume,
        "PATCH",
        &location,
        &[("content-range", "0-9")],
        vec![0; 10],
    )
    .await;
    assert_eq!(stale.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(stale.headers().get("range").is_none(), "as the reference");

    let status = send(&volume, "GET", &location, &[]).await;
    assert_eq!(status.status(), StatusCode::NO_CONTENT);
    assert_eq!(header(&status, "range"), "0-1048575");

    // With Content-Range, as the OCI specification writes it.
    let range = format!("1048576-{}", bytes.len() - 1);
    let patched = send_body(
        &volume,
        "PATCH",
        &location,
        &[("content-range", &range)],
        second.to_vec(),
    )
    .await;
    assert_eq!(patched.status(), StatusCode::ACCEPTED);

    let closed = send(&volume, "PUT", &format!("{location}?digest={digest}"), &[]).await;
    assert_eq!(closed.status(), StatusCode::CREATED);
    assert_eq!(
        header(&closed, "location"),
        format!("/v2/team/app/blobs/{digest}")
    );
    assert_eq!(header(&closed, "docker-content-digest"), digest.as_str());

    let pulled = send(&volume, "GET", &format!("/v2/team/app/blobs/{digest}"), &[]).await;
    assert_eq!(body(pulled).await, bytes);
    let gone = send(&volume, "GET", &location, &[]).await;
    assert_eq!(error_code(gone).await, "BLOB_UPLOAD_UNKNOWN");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_pushed_in_one_request_and_one_closed_with_its_last_chunk() {
    let volume = volume();
    let bytes = layer();
    let digest = Digest::sha256_of(&bytes);
    let monolithic = send_body(
        &volume,
        "POST",
        &format!("/v2/one/shot/blobs/uploads/?digest={digest}"),
        &[],
        bytes.clone(),
    )
    .await;
    assert_eq!(monolithic.status(), StatusCode::CREATED);
    assert_eq!(
        header(&monolithic, "location"),
        format!("/v2/one/shot/blobs/{digest}")
    );

    let location = open_upload(&volume, "two/steps").await;
    let encoded = digest.as_str().replace(':', "%3A");
    let closed = send_body(
        &volume,
        "PUT",
        &format!("{location}?digest={encoded}"),
        &[],
        bytes.clone(),
    )
    .await;
    assert_eq!(closed.status(), StatusCode::CREATED);
    for repo in ["one/shot", "two/steps"] {
        let head = send(&volume, "HEAD", &format!("/v2/{repo}/blobs/{digest}"), &[]).await;
        assert_eq!(head.status(), StatusCode::OK, "{repo}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_digest_the_bytes_do_not_hash_to_is_refused_and_nothing_is_kept() {
    let volume = volume();
    let wrong = Digest::sha256_of(b"something else");
    let location = open_upload(&volume, "team/app").await;
    let closed = send_body(
        &volume,
        "PUT",
        &format!("{location}?digest={wrong}"),
        &[],
        b"the bytes".to_vec(),
    )
    .await;
    assert_eq!(closed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(closed).await, "DIGEST_INVALID");
    // Nothing is kept: not under the digest claimed, and not under the digest
    // the bytes really have.
    for digest in [wrong, Digest::sha256_of(b"the bytes")] {
        let absent = send(
            &volume,
            "HEAD",
            &format!("/v2/team/app/blobs/{digest}"),
            &[],
        )
        .await;
        assert_eq!(absent.status(), StatusCode::NOT_FOUND, "{digest}");
    }

    let location = open_upload(&volume, "team/app").await;
    let missing = send(&volume, "PUT", &location, &[]).await;
    assert_eq!(error_code(missing).await, "DIGEST_INVALID");
    let malformed = send(
        &volume,
        "PUT",
        &format!("{location}?digest=sha256:abc"),
        &[],
    )
    .await;
    assert_eq!(error_code(malformed).await, "DIGEST_INVALID");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mount_cannot_reach_a_blob_the_source_does_not_link() {
    let volume = volume();
    let (digest, _, _) = seed(&volume.store);

    let mounted = send(
        &volume,
        "POST",
        &format!("/v2/other/app/blobs/uploads/?mount={digest}&from=team%2Fapp"),
        &[],
    )
    .await;
    assert_eq!(mounted.status(), StatusCode::CREATED);
    assert_eq!(
        header(&mounted, "location"),
        format!("/v2/other/app/blobs/{digest}")
    );
    let head = send(
        &volume,
        "HEAD",
        &format!("/v2/other/app/blobs/{digest}"),
        &[],
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);

    // The blob exists in the store. Neither of these sources links it, so
    // neither mount may succeed: each opens an ordinary upload instead.
    for query in [
        format!("mount={digest}&from=never%2Fseen"),
        format!("mount={digest}"),
    ] {
        let response = send(
            &volume,
            "POST",
            &format!("/v2/thief/app/blobs/uploads/?{query}"),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED, "{query}");
        let head = send(
            &volume,
            "HEAD",
            &format!("/v2/thief/app/blobs/{digest}"),
            &[],
        )
        .await;
        assert_eq!(head.status(), StatusCode::NOT_FOUND, "{query}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_belongs_to_its_repository_and_can_be_abandoned() {
    let volume = volume();
    let location = open_upload(&volume, "team/app").await;
    let elsewhere = location.replace("team/app", "other/app");
    let response = send(&volume, "GET", &elsewhere, &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(error_code(response).await, "BLOB_UPLOAD_UNKNOWN");

    let cancelled = send(&volume, "DELETE", &location, &[]).await;
    assert_eq!(cancelled.status(), StatusCode::NO_CONTENT);
    let again = send(&volume, "DELETE", &location, &[]).await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_is_stored_byte_for_byte_and_checked_against_its_repository() {
    let volume = volume();
    let (layer_digest, _, _) = seed(&volume.store);
    // Odd spacing on purpose: what comes back must be exactly what went in.
    let manifest = format!(
        "{{ \"schemaVersion\": 2,\n\t\"mediaType\": \"{MANIFEST_TYPE}\",\n \"layers\": [ {{ \"digest\": \"{layer_digest}\" }} ] }}"
    )
    .into_bytes();
    let digest = Digest::sha256_of(&manifest);
    let content_type = [("content-type", MANIFEST_TYPE)];

    let pushed = send_body(
        &volume,
        "PUT",
        "/v2/team/app/manifests/v2",
        &content_type,
        manifest.clone(),
    )
    .await;
    assert_eq!(pushed.status(), StatusCode::CREATED);
    assert_eq!(
        header(&pushed, "location"),
        format!("/v2/team/app/manifests/{digest}")
    );
    assert_eq!(header(&pushed, "docker-content-digest"), digest.as_str());
    let pulled = send(&volume, "GET", "/v2/team/app/manifests/v2", &[]).await;
    assert_eq!(header(&pulled, "content-type"), MANIFEST_TYPE);
    assert_eq!(body(pulled).await, manifest);

    // By a digest the body does not hash to.
    let wrong = Digest::sha256_of(b"another body");
    let refused = send_body(
        &volume,
        "PUT",
        &format!("/v2/team/app/manifests/{wrong}"),
        &content_type,
        manifest.clone(),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(refused).await, "DIGEST_INVALID");

    // Into a repository that does not link the layer.
    let refused = send_body(
        &volume,
        "PUT",
        "/v2/other/app/manifests/v2",
        &content_type,
        manifest,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let value: serde_json::Value = serde_json::from_slice(&body(refused).await).expect("json");
    assert_eq!(value["errors"][0]["code"], "MANIFEST_BLOB_UNKNOWN");
    assert_eq!(value["errors"][0]["detail"], layer_digest.as_str());

    let garbage = send_body(
        &volume,
        "PUT",
        "/v2/team/app/manifests/v3",
        &content_type,
        b"not a manifest".to_vec(),
    )
    .await;
    assert_eq!(error_code(garbage).await, "MANIFEST_INVALID");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_with_a_subject_says_so_and_one_over_the_cap_is_refused() {
    let volume = volume();
    let (_, subject, _) = seed(&volume.store);
    let manifest =
        format!(r#"{{"schemaVersion":2,"layers":[],"subject":{{"digest":"{subject}"}}}}"#)
            .into_bytes();
    let pushed = send_body(
        &volume,
        "PUT",
        "/v2/team/app/manifests/sbom",
        &[("content-type", MANIFEST_TYPE)],
        manifest,
    )
    .await;
    assert_eq!(pushed.status(), StatusCode::CREATED);
    assert_eq!(header(&pushed, "oci-subject"), subject.as_str());

    let huge = send_body(
        &volume,
        "PUT",
        "/v2/team/app/manifests/huge",
        &[("content-type", MANIFEST_TYPE)],
        vec![b' '; (4 << 20) + 1],
    )
    .await;
    assert_eq!(
        huge.status(),
        StatusCode::BAD_REQUEST,
        "as the reference: 400, not 413"
    );
    assert_eq!(error_code(huge).await, "MANIFEST_INVALID");
}

/// FR-R31: with the reference's `http.headers` recipe, a web UI on another
/// origin can preflight, list, and read the headers it needs.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_on_another_origin_can_preflight_and_list() {
    let mut volume = volume();
    volume.headers = vec![
        ("access-control-allow-origin", "https://ui.example"),
        ("access-control-allow-methods", "HEAD, GET, OPTIONS, DELETE"),
        ("access-control-allow-headers", "Authorization, Accept"),
        (
            "access-control-expose-headers",
            "Docker-Content-Digest, Link",
        ),
    ];
    seed(&volume.store);
    let preflight = send(
        &volume,
        "OPTIONS",
        "/v2/team/app/manifests/v1",
        &[
            ("origin", "https://ui.example"),
            ("access-control-request-method", "DELETE"),
        ],
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    assert_eq!(
        header(&preflight, "access-control-allow-origin"),
        "https://ui.example"
    );
    assert!(header(&preflight, "access-control-allow-methods").contains("DELETE"));

    let listed = send(&volume, "GET", "/v2/team/app/tags/list?n=1", &[]).await;
    assert_eq!(
        header(&listed, "access-control-expose-headers"),
        "Docker-Content-Digest, Link"
    );
    // On an error too: a UI must be able to read why it was refused.
    let refused = send(&volume, "GET", "/v2/team/app/manifests/nope", &[]).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        header(&refused, "access-control-allow-origin"),
        "https://ui.example"
    );
}

/// An empty manifest in `name`, so the catalogue counts it.
async fn seed_manifest_in(volume: &Volume, name: &str) {
    let manifest =
        format!(r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_TYPE}","layers":[]}}"#).into_bytes();
    let pushed = send_body(
        volume,
        "PUT",
        &format!("/v2/{name}/manifests/v1"),
        &[("content-type", MANIFEST_TYPE)],
        manifest,
    )
    .await;
    assert_eq!(pushed.status(), StatusCode::CREATED, "{name}");
}

// ---- Discovery and management ------------------------------------------------

async fn json_body(response: Response) -> serde_json::Value {
    serde_json::from_slice(&body(response).await).expect("json")
}

/// Tag the seeded manifest again under each of `tags`.
async fn tag_again(volume: &Volume, manifest: &[u8], tags: &[&str]) {
    for tag in tags {
        let pushed = send_body(
            volume,
            "PUT",
            &format!("/v2/team/app/manifests/{tag}"),
            &[("content-type", MANIFEST_TYPE)],
            manifest.to_vec(),
        )
        .await;
        assert_eq!(pushed.status(), StatusCode::CREATED, "{tag}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tags_are_listed_in_byte_order_and_paged_with_a_link() {
    let volume = volume();
    let (_, _, manifest) = seed(&volume.store);
    tag_again(&volume, &manifest, &["v10", "v2", "alpha"]).await;

    let all = send(&volume, "GET", "/v2/team/app/tags/list", &[]).await;
    assert_eq!(all.status(), StatusCode::OK);
    assert!(all.headers().get("link").is_none());
    let value = json_body(all).await;
    assert_eq!(value["name"], "team/app");
    assert_eq!(
        value["tags"],
        serde_json::json!(["alpha", "v1", "v10", "v2"]),
        "v10 sorts before v2"
    );

    let first = send(&volume, "GET", "/v2/team/app/tags/list?n=2", &[]).await;
    assert_eq!(
        header(&first, "link"),
        "</v2/team/app/tags/list?last=v1&n=2>; rel=\"next\""
    );
    assert_eq!(
        json_body(first).await["tags"],
        serde_json::json!(["alpha", "v1"])
    );
    let second = send(&volume, "GET", "/v2/team/app/tags/list?last=v1&n=2", &[]).await;
    assert!(
        second.headers().get("link").is_none(),
        "the last page has no next"
    );
    assert_eq!(
        json_body(second).await["tags"],
        serde_json::json!(["v10", "v2"])
    );

    let unknown = send(&volume, "GET", "/v2/never/seen/tags/list", &[]).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(error_code(unknown).await, "NAME_UNKNOWN");
    let bad = send(&volume, "GET", "/v2/team/app/tags/list?n=many", &[]).await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(bad).await, "PAGINATION_NUMBER_INVALID");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalogue_lists_repositories_and_pages() {
    let volume = volume();
    seed(&volume.store);
    push_blob(&volume.store, "alpha/one", b"one");
    push_blob(&volume.store, "zeta/last", b"two");
    // Repositories that hold blobs and no manifest are not listed.
    let only_blobs = send(&volume, "GET", "/v2/_catalog", &[]).await;
    assert_eq!(
        json_body(only_blobs).await["repositories"],
        serde_json::json!(["team/app"])
    );
    seed_manifest_in(&volume, "alpha/one").await;
    seed_manifest_in(&volume, "zeta/last").await;
    let all = send(&volume, "GET", "/v2/_catalog", &[]).await;
    assert_eq!(
        json_body(all).await["repositories"],
        serde_json::json!(["alpha/one", "team/app", "zeta/last"])
    );
    let first = send(&volume, "GET", "/v2/_catalog?n=1", &[]).await;
    assert_eq!(
        header(&first, "link"),
        "</v2/_catalog?last=alpha/one&n=1>; rel=\"next\""
    );
    let rest = send(&volume, "GET", "/v2/_catalog?last=alpha%2Fone", &[]).await;
    assert_eq!(
        json_body(rest).await["repositories"],
        serde_json::json!(["team/app", "zeta/last"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn referrers_are_an_index_filtered_by_artifact_type_and_never_404() {
    let volume = volume();
    let (_, subject, _) = seed(&volume.store);
    for (tag, artifact_type) in [
        ("sbom", "application/vnd.example.sbom"),
        ("sig", "application/vnd.example.sig"),
    ] {
        let manifest = format!(
            r#"{{"schemaVersion":2,"artifactType":"{artifact_type}","layers":[],"subject":{{"digest":"{subject}"}},"annotations":{{"made.by":"{tag}"}}}}"#
        );
        let pushed = send_body(
            &volume,
            "PUT",
            &format!("/v2/team/app/manifests/{tag}"),
            &[("content-type", MANIFEST_TYPE)],
            manifest.into_bytes(),
        )
        .await;
        assert_eq!(pushed.status(), StatusCode::CREATED);
    }
    let path = format!("/v2/team/app/referrers/{subject}");
    let all = send(&volume, "GET", &path, &[]).await;
    assert_eq!(all.status(), StatusCode::OK);
    assert_eq!(
        header(&all, "content-type"),
        "application/vnd.oci.image.index.v1+json"
    );
    assert!(all.headers().get("oci-filters-applied").is_none());
    let value = json_body(all).await;
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["manifests"].as_array().expect("manifests").len(), 2);

    let filtered = send(
        &volume,
        "GET",
        &format!("{path}?artifactType=application%2Fvnd.example.sbom"),
        &[],
    )
    .await;
    assert_eq!(header(&filtered, "oci-filters-applied"), "artifactType");
    let value = json_body(filtered).await;
    let manifests = value["manifests"].as_array().expect("manifests");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0]["artifactType"], "application/vnd.example.sbom");
    assert_eq!(manifests[0]["mediaType"], MANIFEST_TYPE);
    assert_eq!(manifests[0]["annotations"]["made.by"], "sbom");
    assert!(manifests[0]["size"].as_u64().expect("size") > 0);

    let nothing = Digest::sha256_of(b"no one refers to this");
    let empty = send(
        &volume,
        "GET",
        &format!("/v2/team/app/referrers/{nothing}"),
        &[],
    )
    .await;
    assert_eq!(empty.status(), StatusCode::OK);
    assert_eq!(json_body(empty).await["manifests"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_is_off_until_it_is_turned_on() {
    let volume = volume();
    let (layer_digest, digest, _) = seed(&volume.store);
    for path in [
        format!("/v2/team/app/manifests/{digest}"),
        format!("/v2/team/app/blobs/{layer_digest}"),
    ] {
        let refused = send(&volume, "DELETE", &path, &[]).await;
        assert_eq!(refused.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        assert_eq!(error_code(refused).await, "UNSUPPORTED");
        let still = send(&volume, "HEAD", &path, &[]).await;
        assert_eq!(still.status(), StatusCode::OK);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_links_in_one_repository_and_leaves_the_other_pulling() {
    let mut volume = volume();
    volume.delete_enabled = true;
    let (layer_digest, digest, manifest) = seed(&volume.store);
    tag_again(&volume, &manifest, &["keep", "drop"]).await;
    // A second repository shares the layer.
    let mounted = send(
        &volume,
        "POST",
        &format!("/v2/other/app/blobs/uploads/?mount={layer_digest}&from=team%2Fapp"),
        &[],
    )
    .await;
    assert_eq!(mounted.status(), StatusCode::CREATED);

    // A tag goes alone; the manifest stays.
    let dropped = send(&volume, "DELETE", "/v2/team/app/manifests/drop", &[]).await;
    assert_eq!(dropped.status(), StatusCode::ACCEPTED);
    let manifest_path = format!("/v2/team/app/manifests/{digest}");
    let by_digest = send(&volume, "HEAD", &manifest_path, &[]).await;
    assert_eq!(by_digest.status(), StatusCode::OK);

    // By digest: the manifest and every tag on it.
    let deleted = send(&volume, "DELETE", &manifest_path, &[]).await;
    assert_eq!(deleted.status(), StatusCode::ACCEPTED);
    for reference in ["keep", "v1", digest.as_str()] {
        let path = format!("/v2/team/app/manifests/{reference}");
        let gone = send(&volume, "GET", &path, &[]).await;
        assert_eq!(gone.status(), StatusCode::NOT_FOUND, "{reference}");
    }
    let again = send(&volume, "DELETE", &manifest_path, &[]).await;
    assert_eq!(error_code(again).await, "MANIFEST_UNKNOWN");

    let blob_path = format!("/v2/team/app/blobs/{layer_digest}");
    let deleted = send(&volume, "DELETE", &blob_path, &[]).await;
    assert_eq!(deleted.status(), StatusCode::ACCEPTED);
    let gone = send(&volume, "HEAD", &blob_path, &[]).await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    let again = send(&volume, "DELETE", &blob_path, &[]).await;
    assert_eq!(error_code(again).await, "BLOB_UNKNOWN");
    let shared = send(
        &volume,
        "GET",
        &format!("/v2/other/app/blobs/{layer_digest}"),
        &[],
    )
    .await;
    assert_eq!(shared.status(), StatusCode::OK);
    assert_eq!(
        body(shared).await,
        layer(),
        "the other repository still pulls"
    );
}

// ---- Through the binary ----------------------------------------------------

mod served {
    use super::{header_of, options, seed};
    use hologram_live::config::AppConfig;
    use hologram_live::oci_store::OciStore;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const TOKEN_ENV: &str = "HOLOGRAM_OCI_HTTP_TEST_TOKEN";
    const TOKEN: &str = "a-token-only-this-test-knows";

    pub struct Server {
        child: Child,
        pub port: u16,
    }

    impl Server {
        /// `hologram stop`, the ordinary way, and wait for the process to end.
        pub fn stop(mut self, root: &Path) {
            let status = Command::new(env!("CARGO_BIN_EXE_hologram"))
                .arg("--config")
                .arg(root.join("config/live.toml"))
                .arg("stop")
                .env("HOME", root)
                .env("USERPROFILE", root)
                .env("HOLOGRAM_CONFIG_DIR", root.join("config"))
                .env(TOKEN_ENV, TOKEN)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("hologram stop");
            assert!(status.success(), "hologram stop failed");
            let deadline = Instant::now() + Duration::from_secs(30);
            while self.child.try_wait().expect("wait").is_none() {
                assert!(Instant::now() < deadline, "the server did not stop");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub struct Answer {
        pub status: u16,
        pub head: String,
        pub body: Vec<u8>,
    }

    /// One HTTP/1.1 exchange, read until the server closes.
    pub fn request(port: u16, method: &str, path: &str, token: bool) -> Answer {
        request_with(port, method, path, token, "application/octet-stream", &[])
    }

    /// As [`request`], with a body.
    pub fn request_with(
        port: u16,
        method: &str,
        path: &str,
        token: bool,
        content_type: &str,
        body: &[u8],
    ) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        let authorization = if token {
            format!("Authorization: Bearer {TOKEN}\r\n")
        } else {
            String::new()
        };
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{authorization}Content-Type: {content_type}\r\nContent-Length: {}\r\nAccept: application/vnd.oci.image.manifest.v1+json\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("send");
        stream.write_all(body).expect("send the body");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read");
        parse(&raw)
    }

    fn parse(raw: &[u8]) -> Answer {
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("end of head");
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("status");
        Answer {
            status,
            head,
            body: raw[split + 4..].to_vec(),
        }
    }

    /// A request the server may refuse, sent the way that keeps its answer.
    ///
    /// A server that answers and closes while the client is still writing
    /// resets the connection, and on macOS the reset takes the unread answer
    /// with it. So the body is offered with `Expect: 100-continue`: a server
    /// that refuses on the head answers before a byte of body is sent, and one
    /// that wants the body says so, reads all of it, and then answers.
    pub fn request_expecting(port: u16, method: &str, path: &str, body: &[u8]) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_mins(1)))
            .expect("timeout");
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nExpect: 100-continue\r\nAccept: application/vnd.oci.image.manifest.v1+json\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("send");
        let mut raw = Vec::new();
        let mut buffer = [0_u8; 4096];
        let end_of_head = |raw: &[u8]| raw.windows(4).position(|window| window == b"\r\n\r\n");
        while end_of_head(&raw).is_none() {
            let n = stream.read(&mut buffer).expect("read the first answer");
            assert!(n > 0, "the server closed without answering");
            raw.extend_from_slice(&buffer[..n]);
        }
        if raw.starts_with(b"HTTP/1.1 100") {
            let after = end_of_head(&raw).expect("end of head") + 4;
            raw.drain(..after);
            stream.write_all(body).expect("send the body");
        }
        stream.read_to_end(&mut raw).expect("read");
        parse(&raw)
    }

    /// Start `hologram serve` on a directory of its own. Every path the binary
    /// could reach for is inside `root`; the configuration is named on the
    /// command line, so no configuration outside the test is ever read.
    pub fn start(root: &Path, with_registry: bool) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("address")
            .port();
        let mut config = AppConfig::default();
        config.paths.config_dir = root.join("config");
        config.paths.data_dir = root.join("data");
        config.paths.state_dir = root.join("state");
        config.paths.cache_dir = root.join("cache");
        config.server.listen = format!("127.0.0.1:{port}");
        // So `hologram stop` reaches this server, not the default address.
        config.client.local_endpoint = format!("http://127.0.0.1:{port}");
        config.auth.required = true;
        TOKEN_ENV.clone_into(&mut config.auth.token_env);
        if with_registry {
            config
                .modules
                .enabled
                .push(hologram_live::modules::oci::MODULE_ID.to_owned());
        }
        std::fs::create_dir_all(root.join("config")).expect("config directory");
        let path = root.join("config/live.toml");
        std::fs::write(&path, toml::to_string_pretty(&config).expect("encode")).expect("write");

        let child = Command::new(env!("CARGO_BIN_EXE_hologram"))
            .arg("--config")
            .arg(&path)
            .arg("serve")
            .env("HOME", root)
            .env("USERPROFILE", root)
            .env("HOLOGRAM_CONFIG_DIR", root.join("config"))
            .env(TOKEN_ENV, TOKEN)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn hologram serve");
        let server = Server { child, port };
        let deadline = Instant::now() + Duration::from_mins(1);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "the server did not start");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }

    #[test]
    fn the_registry_answers_for_itself_and_the_rest_stays_behind_the_token() {
        let root = tempfile::tempdir().expect("tempdir");
        // Seed the volume before the server takes its lock.
        let (layer, manifest, manifest_bytes) = {
            let store = OciStore::open(&root.path().join("data"), options()).expect("open");
            seed(&store)
        };
        let server = start(root.path(), true);

        let base = request(server.port, "GET", "/v2/", false);
        assert_eq!(base.status, 200, "{}", base.head);
        assert_eq!(base.body, b"{}");
        assert_eq!(
            header_of(&base.head, "docker-distribution-api-version"),
            Some("registry/2.0")
        );

        let modules = request(server.port, "GET", "/api/v1/modules", false);
        assert_eq!(modules.status, 401);
        assert!(String::from_utf8_lossy(&modules.body).contains("LIVE_AUTHENTICATION_FAILED"));
        let modules = request(server.port, "GET", "/api/v1/modules", true);
        assert_eq!(modules.status, 200);
        assert!(String::from_utf8_lossy(&modules.body).contains("dev.hologram.live.oci"));

        // Through the real HTTP stack a HEAD keeps the blob's length and sends no body.
        let head = request(
            server.port,
            "HEAD",
            &format!("/v2/team/app/blobs/{layer}"),
            false,
        );
        assert_eq!(head.status, 200, "{}", head.head);
        assert_eq!(header_of(&head.head, "content-length"), Some("3145728"));
        assert!(head.body.is_empty());

        let pulled = request(server.port, "GET", "/v2/team/app/manifests/v1", false);
        assert_eq!(pulled.status, 200);
        assert_eq!(pulled.body, manifest_bytes);
        assert_eq!(
            header_of(&pulled.head, "docker-content-digest"),
            Some(manifest.as_str())
        );

        let blob = request(
            server.port,
            "GET",
            &format!("/v2/team/app/blobs/{layer}"),
            false,
        );
        assert_eq!(blob.status, 200);
        assert_eq!(blob.body.len(), 3 << 20);

        let error = request(server.port, "GET", "/v2/team/app/manifests/nope", false);
        assert_eq!(error.status, 404);
        assert_eq!(
            header_of(&error.head, "docker-distribution-api-version"),
            Some("registry/2.0")
        );
        assert!(String::from_utf8_lossy(&error.body).contains("MANIFEST_UNKNOWN"));
    }

    #[test]
    fn a_layer_larger_than_the_servers_body_limit_goes_in_and_the_limit_still_binds_the_rest() {
        let root = tempfile::tempdir().expect("tempdir");
        let server = start(root.path(), true);
        // One byte over the server-wide limit of 32 MiB.
        let layer = vec![7_u8; (32 << 20) + 1];

        let opened = request_with(
            server.port,
            "POST",
            "/v2/big/layer/blobs/uploads/",
            false,
            "",
            &[],
        );
        assert_eq!(opened.status, 202, "{}", opened.head);
        // Absolute, as the reference answers it; the raw client wants the path.
        let location = header_of(&opened.head, "location")
            .expect("location")
            .strip_prefix(&format!("http://127.0.0.1:{}", server.port))
            .expect("an absolute Location on the address the client used")
            .to_owned();
        let patched = request_with(
            server.port,
            "PATCH",
            &location,
            false,
            "application/octet-stream",
            &layer,
        );
        assert_eq!(patched.status, 202, "{}", patched.head);
        assert_eq!(header_of(&patched.head, "range"), Some("0-33554432"));

        let object = request_expecting(server.port, "POST", "/api/v1/objects", &layer);
        assert_eq!(object.status, 413, "{}", object.head);

        // Finished, the layer is a write the server's audit log records, as
        // the server's own writes are.
        let digest = {
            use sha2::Digest as _;
            let mut hex = String::new();
            for byte in sha2::Sha256::digest(&layer) {
                use std::fmt::Write as _;
                write!(hex, "{byte:02x}").expect("hex");
            }
            format!("sha256:{hex}")
        };
        let finished = request_with(
            server.port,
            "PUT",
            &format!("{location}?digest={digest}"),
            false,
            "application/octet-stream",
            &[],
        );
        assert_eq!(finished.status, 201, "{}", finished.head);
        // The server writes its audit log out when it stops, for the registry's
        // writes as for its own: stop it the ordinary way, then read.
        server.stop(root.path());
        let recorded =
            std::fs::read_to_string(root.path().join("state/audit.jsonl")).unwrap_or_default();
        let line = recorded
            .lines()
            .find(|line| line.contains("oci.blob.put"))
            .unwrap_or_else(|| panic!("no registry write in the audit log: {recorded}"));
        let event: serde_json::Value = serde_json::from_str(line).expect("json line");
        assert_eq!(event["principal"], "anonymous");
        assert_eq!(event["outcome"], "accepted");
        assert_eq!(event["resource"], format!("big/layer@{digest}"));
    }

    #[test]
    fn without_the_module_v2_is_the_servers_own_404() {
        let root = tempfile::tempdir().expect("tempdir");
        let server = start(root.path(), false);
        let answer = request(server.port, "GET", "/v2/", true);
        assert_eq!(answer.status, 404);
        assert!(String::from_utf8_lossy(&answer.body).contains("LIVE_NOT_FOUND"));
        assert_eq!(
            header_of(&answer.head, "docker-distribution-api-version"),
            None
        );
        assert!(
            !root.path().join("data/HOLOGRAM_REGISTRY_LAYOUT").exists(),
            "no volume is made for a module that is off"
        );
    }
}

/// The value of header `name` in a raw response head.
fn header_of<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}
