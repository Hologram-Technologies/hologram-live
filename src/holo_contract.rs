//! Canonical guest-contract selection for executable Wasm layers.

pub use hologram::space::{WASM_CONTRACT_COMPONENT_V1, WASM_CONTRACT_CORE_V1};

pub const COMPONENT_V1_ENTRY: &str = "run";

/// Normalize the legacy empty Wasm tag and the explicit core selector to one
/// provider-facing contract identifier. Identifiers are exact and closed.
pub fn normalize_wasm_contract(value: &str) -> std::result::Result<&'static str, String> {
    match value {
        "" | WASM_CONTRACT_CORE_V1 => Ok(WASM_CONTRACT_CORE_V1),
        WASM_CONTRACT_COMPONENT_V1 => Ok(WASM_CONTRACT_COMPONENT_V1),
        other => Err(format!(
            "unsupported Wasm guest contract {other:?}; expected {WASM_CONTRACT_CORE_V1:?} or {WASM_CONTRACT_COMPONENT_V1:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tag_is_the_core_v1_compatibility_alias() {
        assert_eq!(normalize_wasm_contract(""), Ok(WASM_CONTRACT_CORE_V1));
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_CORE_V1),
            Ok(WASM_CONTRACT_CORE_V1)
        );
    }

    #[test]
    fn component_is_exact_and_unknown_identifiers_fail_closed() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_V1),
            Ok(WASM_CONTRACT_COMPONENT_V1)
        );
        assert!(normalize_wasm_contract("hologram:guest/component@2").is_err());
    }
}
