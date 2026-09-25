//! Per-node cluster identity.
//!
//! A node's name is its ed25519 public key, so membership cannot be spoofed by
//! claiming someone else's identifier and two identically configured hosts are
//! still distinct members. Phase 2 builds the iroh `SecretKey` from the same
//! secret bytes, keeping one identity across the transport change.
//!
//! This task (Task 2 of the distributed-p2p-clustering plan) adds the module
//! without wiring it in: Task 4 (`src/config.rs`) calls `parse_node_id` and
//! Task 5 (`src/app.rs`) calls `NodeIdentity`. Until a later task adds those
//! call sites, every item here is unreachable from the rest of the crate,
//! which `-D warnings` otherwise turns into a build failure. `expect` (rather
//! than `allow`) means the moment everything in this module is wired up and
//! genuinely used, the lint stops firing and this annotation itself becomes a
//! compile error — a reminder to remove it instead of a silently stale allow.
#![expect(dead_code, reason = "wired in by tasks 4 and 5 of this plan")]

use crate::error::{LiveError, Result};
use crate::util::hex;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub const KEY_FILE: &str = "node.key";
const NODE_ID_PREFIX: &str = "ed25519:";

// `SigningKey`'s `Debug` impl deliberately omits the secret key
// (`finish_non_exhaustive()` excludes it), so deriving here cannot leak it.
#[derive(Debug)]
pub struct NodeIdentity {
    signing: SigningKey,
}

impl NodeIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                secure_key_file(path)?;
                let bytes = unhex(text.trim()).ok_or_else(|| {
                    LiveError::Config(format!(
                        "{} is not a valid node key; remove it to generate a new identity",
                        path.display()
                    ))
                })?;
                let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    LiveError::Config(format!(
                        "{} is not a valid node key; remove it to generate a new identity",
                        path.display()
                    ))
                })?;
                Ok(Self {
                    signing: SigningKey::from_bytes(&bytes),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_key_file(path),
            Err(error) => Err(LiveError::io(path, error)),
        }
    }

    pub fn node_id(&self) -> String {
        format!(
            "{NODE_ID_PREFIX}{}",
            hex(self.signing.verifying_key().as_bytes())
        )
    }

    pub fn sign(&self, message: &[u8]) -> String {
        hex(&self.signing.sign(message).to_bytes())
    }

    /// The raw secret. Phase 2 constructs the iroh `SecretKey` from these bytes
    /// so a peer dials exactly the identity it already admitted.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }
}

fn create_key_file(path: &Path) -> Result<NodeIdentity> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| LiveError::Io(format!("generate node key: {error}")))?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            let encoded = hex(&secret);
            file.write_all(encoded.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.sync_all())
                .map_err(|error| LiveError::io(path, error))?;
            Ok(NodeIdentity {
                signing: SigningKey::from_bytes(&secret),
            })
        }
        // Another process won the race; read what it wrote.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            NodeIdentity::load_or_create(path)
        }
        Err(error) => Err(LiveError::io(path, error)),
    }
}

#[cfg(unix)]
fn secure_key_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|error| LiveError::io(path, error))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| LiveError::io(path, error))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_key_file(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn parse_node_id(node_id: &str) -> Result<VerifyingKey> {
    let encoded = node_id.strip_prefix(NODE_ID_PREFIX).ok_or_else(|| {
        LiveError::Protocol(format!("cluster node id must start with {NODE_ID_PREFIX}"))
    })?;
    let bytes = unhex(encoded)
        .ok_or_else(|| LiveError::Protocol("cluster node id is not hexadecimal".to_owned()))?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| LiveError::Protocol("cluster node id must be 32 bytes".to_owned()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| LiveError::Protocol("cluster node id is not a valid public key".to_owned()))
}

pub fn verify_signature(node_id: &str, message: &[u8], signature: &str) -> Result<()> {
    let key = parse_node_id(node_id)?;
    let bytes = unhex(signature).ok_or_else(|| {
        LiveError::Authentication("cluster signature is not hexadecimal".to_owned())
    })?;
    let bytes: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| LiveError::Authentication("cluster signature must be 64 bytes".to_owned()))?;
    key.verify(message, &Signature::from_bytes(&bytes))
        .map_err(|_| LiveError::Authentication("invalid cluster signature".to_owned()))
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks(2) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        // Each nibble is 0..=15, so the combined value always fits in a u8;
        // `try_from` proves that to clippy instead of an `as` truncation.
        bytes.push(u8::try_from((high << 4) | low).ok()?);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_and_is_private() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(KEY_FILE);

        let first = NodeIdentity::load_or_create(&path).expect("create identity");
        let second = NodeIdentity::load_or_create(&path).expect("reload identity");

        assert_eq!(first.node_id(), second.node_id());
        assert!(first.node_id().starts_with("ed25519:"));
        assert_eq!(first.node_id().len(), "ed25519:".len() + 64);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("key metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    // Defect 1: two nodes with byte-identical configuration must still be
    // distinct members. The old derived server_id made them the same node.
    #[test]
    fn two_nodes_with_identical_configuration_have_distinct_identities() {
        let first_dir = tempfile::tempdir().expect("first state directory");
        let second_dir = tempfile::tempdir().expect("second state directory");
        let first = NodeIdentity::load_or_create(&first_dir.path().join(KEY_FILE))
            .expect("first identity");
        let second = NodeIdentity::load_or_create(&second_dir.path().join(KEY_FILE))
            .expect("second identity");
        assert_ne!(first.node_id(), second.node_id());
    }

    #[test]
    fn a_signature_verifies_only_for_its_own_message_and_signer() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let identity =
            NodeIdentity::load_or_create(&directory.path().join(KEY_FILE)).expect("identity");
        let signature = identity.sign(b"cluster message");

        verify_signature(&identity.node_id(), b"cluster message", &signature)
            .expect("valid signature");
        assert!(verify_signature(&identity.node_id(), b"other message", &signature).is_err());

        let other_dir = tempfile::tempdir().expect("other state directory");
        let other =
            NodeIdentity::load_or_create(&other_dir.path().join(KEY_FILE)).expect("other identity");
        assert!(verify_signature(&other.node_id(), b"cluster message", &signature).is_err());
    }

    // Review Focus 1: a truncated key must fail loudly, never regenerate.
    #[test]
    fn a_corrupt_key_file_is_a_configuration_error() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(KEY_FILE);
        std::fs::write(&path, "not a key\n").expect("write corrupt key");

        let error = NodeIdentity::load_or_create(&path).expect_err("corrupt key must fail");
        assert!(matches!(error, LiveError::Config(_)), "got {error:?}");
        assert!(
            !error.to_string().contains("not a key"),
            "the error must not echo key file contents"
        );
    }

    #[test]
    fn a_malformed_node_id_is_rejected() {
        assert!(parse_node_id("ed25519:zz").is_err());
        // "4" repeated 64 times decodes to 32 identical bytes that are not a
        // valid compressed Edwards y-coordinate (no x exists for that y mod
        // p). This is deliberately not "a" repeated: that decodes to a valid,
        // full-order ed25519-dalek 3.0.0 public key (verified is_weak() ==
        // false), so it does not exercise this rejection path.
        assert!(parse_node_id(&format!("ed25519:{}", "4".repeat(64))).is_err());
        assert!(parse_node_id("blake3:0123").is_err());
    }
}
