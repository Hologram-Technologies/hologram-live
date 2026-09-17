use hologram_model_hub::hub::{handle, Artifact, Artifacts, Job, JobKind, JobState, Page};
use serde_json::{json, Value};

/// Records every host call and answers from a script.
#[derive(Default)]
struct FakeHost {
    calls: Vec<String>,
    fail: Option<String>,
}

impl FakeHost {
    fn failing(message: &str) -> Self {
        Self {
            calls: Vec::new(),
            fail: Some(message.to_owned()),
        }
    }

    fn answer<T>(&mut self, call: String, value: T) -> Result<T, String> {
        self.calls.push(call);
        self.fail.clone().map_or(Ok(value), Err)
    }
}

const ARCHIVE: &str = "blake3:1111111111111111111111111111111111111111111111111111111111111111";

impl Artifacts for FakeHost {
    fn list(&mut self, cursor: Option<&str>, limit: u16) -> Result<Page, String> {
        let page = Page {
            artifacts: vec![Artifact {
                reference: "models/demo:latest".to_owned(),
                manifest_digest: Some("sha256:abc".to_owned()),
                archive_kappa: ARCHIVE.to_owned(),
                layers: 3,
                bytes: 1024,
                recorded_at_millis: 1,
            }],
            next_cursor: None,
        };
        self.answer(format!("list {cursor:?} {limit}"), page)
    }

    fn pull(&mut self, reference: &str) -> Result<String, String> {
        self.answer(format!("pull {reference}"), "job-1".to_owned())
    }

    fn verify(&mut self, archive_kappa: &str) -> Result<String, String> {
        self.answer(format!("verify {archive_kappa}"), "job-2".to_owned())
    }

    fn status(&mut self, job_id: &str) -> Result<Job, String> {
        let job = Job {
            id: job_id.to_owned(),
            kind: JobKind::Verify,
            state: JobState::Failed,
            layers_done: 2,
            layers_total: 3,
            bytes_done: 512,
            mismatch: Some(ARCHIVE.to_owned()),
            error: Some("layer_mismatch".to_owned()),
        };
        self.answer(format!("status {job_id}"), job)
    }

    fn cancel(&mut self, job_id: &str) -> Result<(), String> {
        self.answer(format!("cancel {job_id}"), ())
    }

    fn remove(&mut self, archive_kappa: &str) -> Result<(), String> {
        self.answer(format!("remove {archive_kappa}"), ())
    }
}

fn call(host: &mut FakeHost, request: &Value) -> Value {
    let bytes = handle(host, request.to_string().as_bytes());
    serde_json::from_slice(&bytes).expect("answers are JSON")
}

#[test]
fn library_forwards_a_bounded_page_and_reports_host_data_unchanged() {
    let mut host = FakeHost::default();
    let answer = call(&mut host, &json!({ "op": "library" }));
    assert_eq!(host.calls, ["list None 50"]);
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["data"]["artifacts"][0]["archive_kappa"], ARCHIVE);
    assert_eq!(answer["data"]["artifacts"][0]["layers"], 3);

    for limit in [0, 257] {
        let answer = call(&mut host, &json!({ "op": "library", "limit": limit }));
        assert_eq!(answer, json!({ "ok": false, "error": "invalid_request" }));
    }
    assert_eq!(host.calls.len(), 1, "invalid pages never reach the host");
}

#[test]
fn pull_trims_and_rejects_malformed_references_before_the_host() {
    let mut host = FakeHost::default();
    let answer = call(
        &mut host,
        &json!({ "op": "pull", "reference": "  qwen3.5:4b \n" }),
    );
    assert_eq!(answer, json!({ "ok": true, "data": { "job": "job-1" } }));
    assert_eq!(host.calls, ["pull qwen3.5:4b"]);

    let long = "a".repeat(513);
    for reference in ["", "   ", "models/a b:1", "models/a\u{7}:1", long.as_str()] {
        let answer = call(&mut host, &json!({ "op": "pull", "reference": reference }));
        assert_eq!(
            answer,
            json!({ "ok": false, "error": "invalid_reference" }),
            "{reference:?}"
        );
    }
    assert_eq!(host.calls.len(), 1);
}

#[test]
fn verify_and_remove_accept_only_blake3_archive_identities() {
    let mut host = FakeHost::default();
    assert_eq!(
        call(&mut host, &json!({ "op": "verify", "archive": ARCHIVE }))["data"]["job"],
        "job-2"
    );
    assert_eq!(
        call(&mut host, &json!({ "op": "remove", "archive": ARCHIVE })),
        json!({ "ok": true, "data": null })
    );

    let upper = ARCHIVE.to_uppercase();
    let short = &ARCHIVE[..70];
    for archive in ["sha256:11", upper.as_str(), short, "blake3:zz"] {
        for op in ["verify", "remove"] {
            let answer = call(&mut host, &json!({ "op": op, "archive": archive }));
            assert_eq!(
                answer,
                json!({ "ok": false, "error": "invalid_archive" }),
                "{op} {archive}"
            );
        }
    }
    assert_eq!(host.calls.len(), 2);
}

#[test]
fn status_reports_the_host_mismatch_exactly() {
    let mut host = FakeHost::default();
    let answer = call(&mut host, &json!({ "op": "status", "job": "job-2" }));
    assert_eq!(answer["data"]["state"], "failed");
    assert_eq!(answer["data"]["mismatch"], ARCHIVE);
    assert_eq!(answer["data"]["layers_done"], 2);
    assert_eq!(
        call(&mut host, &json!({ "op": "cancel", "job": "job-2" }))["ok"],
        true
    );
    assert_eq!(
        call(&mut host, &json!({ "op": "status", "job": "" }))["error"],
        "invalid_request"
    );
}

#[test]
fn host_failures_become_closed_codes_and_never_leak_messages() {
    for (message, code) in [
        ("denied: registry.internal:5000/secret-namespace", "denied"),
        ("not_found", "not_found"),
        ("busy", "busy"),
        ("conflict: job running", "conflict"),
        ("invalid_reference", "invalid_reference"),
        ("socket closed at 10.0.0.7", "failed"),
    ] {
        let mut host = FakeHost::failing(message);
        let bytes = handle(
            &mut host,
            json!({ "op": "pull", "reference": "models/a:1" })
                .to_string()
                .as_bytes(),
        );
        let text = String::from_utf8(bytes).expect("utf-8");
        assert_eq!(
            serde_json::from_str::<Value>(&text).expect("json"),
            json!({ "ok": false, "error": code })
        );
        assert!(
            !text.contains("secret") && !text.contains("10.0.0.7"),
            "{text}"
        );
    }
}

#[test]
fn malformed_requests_are_rejected_without_host_calls() {
    let mut host = FakeHost::default();
    for input in [
        b"not json".as_slice(),
        br#"{"op":"delete_everything"}"#,
        br#"{"op":"pull"}"#,
        br#"{"op":"pull","reference":"a:1","scope":"*"}"#,
        br"[]",
    ] {
        let answer: Value = serde_json::from_slice(&handle(&mut host, input)).expect("json");
        assert_eq!(answer, json!({ "ok": false, "error": "invalid_request" }));
    }
    assert!(host.calls.is_empty());
}
