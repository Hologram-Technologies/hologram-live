//! Model Hub: a portable View over named artifacts (ADR 022), using the host
//! artifact interface proposed in ADR 023.

pub mod hub;

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::pedantic)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "application",
        generate_all,
    });
}

// `export!` expands wit-bindgen's generated export glue here; no hand-written unsafe.
#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod component {
    use super::bindings::exports::hologram::application_artifacts::guest::{
        ErrorCode, Guest, GuestError,
    };
    use super::bindings::hologram::host::artifacts as host;
    use super::hub;

    struct Host;

    fn artifact(a: host::Artifact) -> hub::Artifact {
        hub::Artifact {
            reference: a.reference,
            manifest_digest: a.manifest_digest,
            archive_kappa: a.archive_kappa,
            layers: a.layers,
            bytes: a.bytes,
            recorded_at_millis: a.recorded_at_millis,
        }
    }

    impl hub::Artifacts for Host {
        fn list(&mut self, cursor: Option<&str>, limit: u16) -> Result<hub::Page, String> {
            host::list(cursor, limit).map(|page| hub::Page {
                artifacts: page.artifacts.into_iter().map(artifact).collect(),
                next_cursor: page.next_cursor,
            })
        }

        fn pull(&mut self, reference: &str) -> Result<String, String> {
            host::pull(reference)
        }

        fn verify(&mut self, archive_kappa: &str) -> Result<String, String> {
            host::verify(archive_kappa)
        }

        fn status(&mut self, job_id: &str) -> Result<hub::Job, String> {
            host::status(job_id).map(|job| hub::Job {
                id: job.id,
                kind: match job.kind {
                    host::JobKind::Pull => hub::JobKind::Pull,
                    host::JobKind::Verify => hub::JobKind::Verify,
                },
                state: match job.state {
                    host::JobState::Running => hub::JobState::Running,
                    host::JobState::Succeeded => hub::JobState::Succeeded,
                    host::JobState::Failed => hub::JobState::Failed,
                    host::JobState::Cancelled => hub::JobState::Cancelled,
                },
                layers_done: job.layers_done,
                layers_total: job.layers_total,
                bytes_done: job.bytes_done,
                mismatch: job.mismatch,
                error: job.error,
            })
        }

        fn cancel(&mut self, job_id: &str) -> Result<(), String> {
            host::cancel(job_id)
        }

        fn remove(&mut self, archive_kappa: &str) -> Result<(), String> {
            host::remove(archive_kappa)
        }
    }

    struct ModelHub;

    impl Guest for ModelHub {
        fn run(input: Vec<u8>) -> Result<Vec<u8>, GuestError> {
            if input.is_empty() {
                return Err(GuestError {
                    code: ErrorCode::InvalidInput,
                    message: "empty request".to_owned(),
                });
            }
            Ok(hub::handle(&mut Host, &input))
        }
    }

    super::bindings::export!(ModelHub with_types_in super::bindings);
}
