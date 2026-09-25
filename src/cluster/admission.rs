//! Who may participate in this cluster.
//!
//! This is the seam between a closed cluster and an open network. A future
//! capability-grant implementation satisfies the same trait without touching
//! transport or membership.
//!
//! This task (Task 4 of the distributed-p2p-clustering plan) adds the module
//! without wiring it in: Task 5 (`src/app.rs`) calls `build`. Until that call
//! site lands, everything here outside `#[cfg(test)]` is unreachable from the
//! rest of the crate, which `-D warnings` otherwise turns into a build
//! failure. `expect` (rather than `allow`) means the moment Task 5 wires this
//! in, the lint stops firing and this annotation itself becomes a compile
//! error — a reminder to remove it instead of a silently stale allow.
#![expect(dead_code, reason = "wired in by task 5 of this plan")]

use crate::cluster::identity::parse_node_id;
use crate::error::{LiveError, Result};
use crate::util::{atomic_write, constant_time_eq, hex};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TICKET_CONTEXT: &str = "dev.hologram.live.cluster-ticket.v1";
pub const PINNED_FILE: &str = "cluster-pinned.json";

#[derive(Debug)]
pub enum Decision {
    Admit,
    Deny(String),
}

pub trait Admission: Send + Sync {
    fn authorize(&self, node_id: &str, ticket: Option<&str>) -> Decision;
    /// The identities currently eligible to own resources.
    fn admitted(&self) -> BTreeSet<String>;
}

/// A ticket proves the bearer holds the cluster token *for this identity*.
pub fn ticket(token: &str, node_id: &str) -> String {
    let key = blake3::derive_key(TICKET_CONTEXT, token.as_bytes());
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(node_id.as_bytes());
    hex(hasher.finalize().as_bytes())
}

pub struct AllowlistAdmission {
    trusted: BTreeSet<String>,
}

impl AllowlistAdmission {
    pub fn new(trusted: Vec<String>) -> Self {
        Self {
            trusted: trusted.into_iter().collect(),
        }
    }
}

impl Admission for AllowlistAdmission {
    fn authorize(&self, node_id: &str, _ticket: Option<&str>) -> Decision {
        if self.trusted.contains(node_id) {
            Decision::Admit
        } else {
            Decision::Deny("node is not in cluster.trusted_keys".to_owned())
        }
    }

    fn admitted(&self) -> BTreeSet<String> {
        self.trusted.clone()
    }
}

pub struct TokenAdmission {
    token: String,
    path: PathBuf,
    pinned: Mutex<BTreeSet<String>>,
}

impl TokenAdmission {
    pub fn new(token: String, path: PathBuf, trusted: Vec<String>) -> Result<Self> {
        let mut pinned: BTreeSet<String> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => return Err(LiveError::io(&path, error)),
        };
        pinned.extend(trusted);
        Ok(Self {
            token,
            path,
            pinned: Mutex::new(pinned),
        })
    }
}

impl Admission for TokenAdmission {
    fn authorize(&self, node_id: &str, presented: Option<&str>) -> Decision {
        let Ok(mut pinned) = self.pinned.lock() else {
            return Decision::Deny("admission state is poisoned".to_owned());
        };
        if pinned.contains(node_id) {
            return Decision::Admit;
        }
        if parse_node_id(node_id).is_err() {
            return Decision::Deny("node id is not a valid public key".to_owned());
        }
        let Some(presented) = presented else {
            return Decision::Deny("unknown node presented no admission ticket".to_owned());
        };
        let expected = ticket(&self.token, node_id);
        if !constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
            return Decision::Deny("admission ticket is invalid for this node".to_owned());
        }
        pinned.insert(node_id.to_owned());
        // A failed write must not admit silently on the next restart; log and
        // keep the in-memory pin so this round still works.
        match serde_json::to_vec_pretty(&*pinned)
            .map_err(LiveError::from)
            .and_then(|bytes| atomic_write(&self.path, &bytes))
        {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error, "failed to persist pinned cluster identity"),
        }
        Decision::Admit
    }

    fn admitted(&self) -> BTreeSet<String> {
        self.pinned
            .lock()
            .map(|pinned| pinned.clone())
            .unwrap_or_default()
    }
}

pub fn build(
    config: &crate::config::ClusterConfig,
    token: Option<&str>,
    state_dir: &Path,
) -> Result<Arc<dyn Admission>> {
    match config.admission.as_str() {
        "allowlist" => Ok(Arc::new(AllowlistAdmission::new(
            config.trusted_keys.clone(),
        ))),
        "token" => {
            let token = token
                .ok_or_else(|| {
                    LiveError::Config(
                        "cluster.admission \"token\" requires a cluster token".to_owned(),
                    )
                })?
                .to_owned();
            Ok(Arc::new(TokenAdmission::new(
                token,
                state_dir.join(PINNED_FILE),
                config.trusted_keys.clone(),
            )?))
        }
        other => Err(LiveError::Config(format!(
            "unsupported cluster.admission {other:?}; expected token or allowlist"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const BOB: &str = "ed25519:3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

    #[test]
    fn the_allowlist_admits_only_listed_identities() {
        let admission = AllowlistAdmission::new(vec![ALICE.to_owned()]);
        assert!(matches!(admission.authorize(ALICE, None), Decision::Admit));
        assert!(matches!(admission.authorize(BOB, None), Decision::Deny(_)));
    }

    #[test]
    fn a_valid_ticket_pins_a_new_identity_and_survives_reload() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join("cluster-pinned.json");
        let token = "a sufficiently long shared cluster admission token";

        let admission =
            TokenAdmission::new(token.to_owned(), path.clone(), Vec::new()).expect("admission");
        // Without a ticket an unknown identity is refused.
        assert!(matches!(admission.authorize(ALICE, None), Decision::Deny(_)));
        // With the right ticket it is admitted, and pinned.
        assert!(matches!(
            admission.authorize(ALICE, Some(&ticket(token, ALICE))),
            Decision::Admit
        ));
        // Pinned identities no longer need the ticket.
        assert!(matches!(admission.authorize(ALICE, None), Decision::Admit));

        let reloaded = TokenAdmission::new(token.to_owned(), path, Vec::new()).expect("reload");
        assert!(matches!(reloaded.authorize(ALICE, None), Decision::Admit));
    }

    #[test]
    fn a_ticket_for_one_identity_does_not_admit_another() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let token = "a sufficiently long shared cluster admission token";
        let admission = TokenAdmission::new(
            token.to_owned(),
            directory.path().join("cluster-pinned.json"),
            Vec::new(),
        )
        .expect("admission");

        // Alice's ticket presented by Bob.
        assert!(matches!(
            admission.authorize(BOB, Some(&ticket(token, ALICE))),
            Decision::Deny(_)
        ));
        assert!(matches!(
            admission.authorize(ALICE, Some("not a ticket")),
            Decision::Deny(_)
        ));
    }
}
