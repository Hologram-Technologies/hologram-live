//! Strict admission for the current physical `.holo` format.

use crate::error::{LiveError, Result};

pub const CURRENT_HOLO_VERSION: u16 = 4;

/// Reject every physical archive version except the one emitted by this build.
pub fn require_current(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 6 || !bytes.starts_with(b"HOLO") {
        return Err(LiveError::InvalidHolo(
            "archive is missing the Hologram header".to_owned(),
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != CURRENT_HOLO_VERSION {
        return Err(LiveError::InvalidHolo(format!(
            "unsupported .holo format version {version}; expected {CURRENT_HOLO_VERSION}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_archive_version_is_admitted() {
        assert!(require_current(b"HOLO\x04\x00").is_ok());
        let error = require_current(b"HOLO\x03\x00").expect_err("old version");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
        assert!(error.to_string().contains("expected 4"));
        assert!(require_current(b"HOLO").is_err());
    }
}
