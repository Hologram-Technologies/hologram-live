//! Deterministic ownership for mutable distributed state.
//!
//! Immutable bytes can converge independently. Mutable names need one
//! authority at a time, so every node derives the same owner from the current
//! live member set and a stable resource key.

use crate::protocol::NodeRecord;

/// Picks the rendezvous-hash owner for `resource`.
///
/// Nodes with no reachable endpoint are excluded. Ties are resolved by node
/// id, making selection independent of discovery order.
pub fn owner<'a>(resource: &str, nodes: &'a [NodeRecord]) -> Option<&'a NodeRecord> {
    nodes
        .iter()
        .filter(|node| !node.node_id.is_empty() && !node.endpoint.is_empty())
        .min_by_key(|node| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(resource.as_bytes());
            hasher.update(&[0]);
            hasher.update(node.node_id.as_bytes());
            *hasher.finalize().as_bytes()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> NodeRecord {
        NodeRecord {
            node_id: id.to_owned(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: format!("https://{id}.example"),
            last_seen_millis: 0,
        }
    }

    #[test]
    fn ownership_is_independent_of_discovery_order() {
        let first = vec![node("alpha"), node("bravo"), node("charlie")];
        let mut second = first.clone();
        second.reverse();
        assert_eq!(
            owner("file:report", &first).unwrap().node_id,
            owner("file:report", &second).unwrap().node_id
        );
    }

    #[test]
    fn unreachable_members_cannot_own_state() {
        let mut unreachable = node("unreachable");
        unreachable.endpoint.clear();
        assert_eq!(
            owner("history:thread", &[unreachable, node("live")])
                .unwrap()
                .node_id,
            "live"
        );
    }
}
