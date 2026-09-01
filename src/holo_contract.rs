//! Canonical guest-contract selection for executable Wasm layers.

pub use hologram::space::{
    WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1, WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1,
    WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1, WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1,
    WASM_CONTRACT_COMPONENT_STORE_READ_V1, WASM_CONTRACT_COMPONENT_STORE_WRITE_V1,
    WASM_CONTRACT_COMPONENT_V1, WASM_CONTRACT_CORE_V1,
};

pub const COMPONENT_V1_ENTRY: &str = "run";

/// Validate one exact provider-facing Wasm contract identifier.
pub fn normalize_wasm_contract(value: &str) -> std::result::Result<&'static str, String> {
    match value {
        WASM_CONTRACT_CORE_V1 => Ok(WASM_CONTRACT_CORE_V1),
        WASM_CONTRACT_COMPONENT_V1 => Ok(WASM_CONTRACT_COMPONENT_V1),
        WASM_CONTRACT_COMPONENT_STORE_READ_V1 => Ok(WASM_CONTRACT_COMPONENT_STORE_READ_V1),
        WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1 => {
            Ok(WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1)
        }
        WASM_CONTRACT_COMPONENT_STORE_WRITE_V1 => Ok(WASM_CONTRACT_COMPONENT_STORE_WRITE_V1),
        WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1 => {
            Ok(WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1)
        }
        WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1 => {
            Ok(WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1)
        }
        WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1 => Ok(WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1),
        other => Err(format!(
            "unsupported Wasm guest contract {other:?}; expected {WASM_CONTRACT_CORE_V1:?}, {WASM_CONTRACT_COMPONENT_V1:?}, {WASM_CONTRACT_COMPONENT_STORE_READ_V1:?}, {WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1:?}, {WASM_CONTRACT_COMPONENT_STORE_WRITE_V1:?}, {WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1:?}, {WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1:?}, or {WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_contract_must_be_explicit() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_CORE_V1),
            Ok(WASM_CONTRACT_CORE_V1)
        );
        assert!(normalize_wasm_contract("").is_err());
    }

    #[test]
    fn component_is_exact_and_unknown_identifiers_fail_closed() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_V1),
            Ok(WASM_CONTRACT_COMPONENT_V1)
        );
        assert!(normalize_wasm_contract("hologram:guest/component@2").is_err());
    }

    #[test]
    fn component_store_read_profile_is_exact() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_STORE_READ_V1),
            Ok(WASM_CONTRACT_COMPONENT_STORE_READ_V1)
        );
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1),
            Ok(WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1)
        );
    }

    #[test]
    fn component_store_write_profile_is_exact() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_STORE_WRITE_V1),
            Ok(WASM_CONTRACT_COMPONENT_STORE_WRITE_V1)
        );
    }

    #[test]
    fn component_channel_profiles_are_exact() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1),
            Ok(WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1)
        );
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1),
            Ok(WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1)
        );
    }

    #[test]
    fn component_network_fetch_profile_is_exact() {
        assert_eq!(
            normalize_wasm_contract(WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1),
            Ok(WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1)
        );
    }
}
