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
    owner_for_operation(resource, nodes, None)
}

/// Picks a rendezvous-hash owner that advertises `required_operation`.
///
/// The caller can use this for read or execution placement before it forwards
/// a request. `None` retains the ordinary mutable-state ownership behavior.
pub fn owner_for_operation<'a>(
    resource: &str,
    nodes: &'a [NodeRecord],
    required_operation: Option<&str>,
) -> Option<&'a NodeRecord> {
    nodes
        .iter()
        .filter(|node| {
            !node.node_id.is_empty()
                && !node.endpoint.is_empty()
                && required_operation.is_none_or(|operation| {
                    node.operations
                        .iter()
                        .any(|advertised| advertised == operation)
                })
        })
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

    #[test]
    fn resource_key_changes_the_assignment() {
        let nodes = vec![node("alpha"), node("bravo"), node("charlie")];
        let owners = (0..64)
            .map(|index| {
                owner(&format!("file:{index}"), &nodes)
                    .unwrap()
                    .node_id
                    .clone()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert!(owners.len() > 1, "ownership should spread independent keys");
    }

    #[test]
    fn placement_requires_the_advertised_operation() {
        let mut capable = node("capable");
        capable.operations.push("holo.run".to_owned());
        let nodes = [node("ineligible"), capable];
        let selected =
            owner_for_operation("holo:sample", &nodes, Some("holo.run")).expect("capable node");
        assert_eq!(selected.node_id, "capable");
    }
}
