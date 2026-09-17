//! Model Hub's primary: one JSON request in, one JSON answer out.
//!
//! The primary holds no state and decides nothing about trust. Pulling,
//! re-hashing, admission and audit belong to the host (ADR 023). This module
//! only validates requests, forwards them to the host, and reports the host's
//! answers unchanged, so the View never shows a status the host did not give.

use serde::{Deserialize, Serialize};

/// Largest reference the host accepts (ADR 023 host ceilings).
pub const MAX_REFERENCE_BYTES: usize = 512;
/// Largest page the host returns (ADR 023 host ceilings).
pub const MAX_PAGE: u16 = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Artifact {
    pub reference: String,
    pub manifest_digest: Option<String>,
    pub archive_kappa: String,
    pub layers: u32,
    pub bytes: u64,
    pub recorded_at_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub artifacts: Vec<Artifact>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Pull,
    Verify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub state: JobState,
    pub layers_done: u32,
    pub layers_total: u32,
    pub bytes_done: u64,
    pub mismatch: Option<String>,
    pub error: Option<String>,
}

/// The host operations Model Hub uses. Mirrors `hologram:host/artifacts@1.0.0`
/// so the logic here is testable without a runtime. Every error is the host's
/// failure string, which [`handle`] reduces to a closed code.
#[allow(clippy::missing_errors_doc)]
pub trait Artifacts {
    fn list(&mut self, cursor: Option<&str>, limit: u16) -> Result<Page, String>;
    fn pull(&mut self, reference: &str) -> Result<String, String>;
    fn verify(&mut self, archive_kappa: &str) -> Result<String, String>;
    fn status(&mut self, job_id: &str) -> Result<Job, String>;
    fn cancel(&mut self, job_id: &str) -> Result<(), String>;
    fn remove(&mut self, archive_kappa: &str) -> Result<(), String>;
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Library {
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        limit: Option<u16>,
    },
    Pull {
        reference: String,
    },
    Verify {
        archive: String,
    },
    Status {
        job: String,
    },
    Cancel {
        job: String,
    },
    Remove {
        archive: String,
    },
}

/// Closed set of error codes the View can explain. Host messages are never
/// shown: they may name registries or scopes (ADR 023 redaction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidRequest,
    InvalidReference,
    InvalidArchive,
    Denied,
    NotFound,
    Busy,
    Conflict,
    Failed,
}

impl Code {
    /// Map a host failure string to a code. Unknown strings become `Failed`.
    fn from_host(message: &str) -> Self {
        match message.split([':', ' ']).next().unwrap_or_default() {
            "denied" => Self::Denied,
            "not_found" => Self::NotFound,
            "invalid_reference" => Self::InvalidReference,
            "busy" => Self::Busy,
            "conflict" => Self::Conflict,
            _ => Self::Failed,
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum Answer<T: Serialize> {
    Ok { ok: bool, data: T },
    Err { ok: bool, error: Code },
}

fn ok<T: Serialize>(data: T) -> Vec<u8> {
    encode(&Answer::Ok { ok: true, data })
}

fn err(code: Code) -> Vec<u8> {
    encode(&Answer::<()>::Err {
        ok: false,
        error: code,
    })
}

fn encode<T: Serialize>(answer: &Answer<T>) -> Vec<u8> {
    // Every answer type is plain data; serialization cannot fail.
    serde_json::to_vec(answer).unwrap_or_else(|_| br#"{"ok":false,"error":"failed"}"#.to_vec())
}

fn answer<T: Serialize>(result: Result<T, String>) -> Vec<u8> {
    result.map_or_else(|message| err(Code::from_host(&message)), ok)
}

/// A reference is non-empty, bounded, and has no whitespace or control bytes.
/// Grammar and scope are the host's to check (ADR 022, ADR 023).
fn valid_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= MAX_REFERENCE_BYTES
        && !reference
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

/// An archive identity is `blake3:` followed by 64 lowercase hex digits.
fn valid_archive(archive: &str) -> bool {
    archive.strip_prefix("blake3:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn valid_job(job: &str) -> bool {
    !job.is_empty() && job.len() <= 128 && job.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Handle one intent payload.
pub fn handle(host: &mut dyn Artifacts, input: &[u8]) -> Vec<u8> {
    let Ok(request) = serde_json::from_slice::<Request>(input) else {
        return err(Code::InvalidRequest);
    };
    match request {
        Request::Library { cursor, limit } => {
            let limit = limit.unwrap_or(50);
            if limit == 0 || limit > MAX_PAGE || cursor.as_deref().is_some_and(|c| !valid_job(c)) {
                return err(Code::InvalidRequest);
            }
            answer(host.list(cursor.as_deref(), limit))
        }
        Request::Pull { reference } => {
            let reference = reference.trim();
            if !valid_reference(reference) {
                return err(Code::InvalidReference);
            }
            answer(
                host.pull(reference)
                    .map(|job| serde_json::json!({ "job": job })),
            )
        }
        Request::Verify { archive } => {
            if !valid_archive(&archive) {
                return err(Code::InvalidArchive);
            }
            answer(
                host.verify(&archive)
                    .map(|job| serde_json::json!({ "job": job })),
            )
        }
        Request::Status { job } if valid_job(&job) => answer(host.status(&job)),
        Request::Cancel { job } if valid_job(&job) => answer(host.cancel(&job)),
        Request::Status { .. } | Request::Cancel { .. } => err(Code::InvalidRequest),
        Request::Remove { archive } => {
            if !valid_archive(&archive) {
                return err(Code::InvalidArchive);
            }
            answer(host.remove(&archive))
        }
    }
}
