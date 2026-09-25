//! Deterministic ownership for mutable distributed state.
//!
//! Immutable bytes can converge independently. Mutable names need one
//! authority at a time, so every node derives the same owner from the current
//! live member set and a stable resource key.

use crate::protocol::NodeRecord;
use std::collections::BTreeSet;

/// Picks the rendezvous-hash owner for `resource`.
///
/// Nodes with no reachable endpoint are excluded. Ties are resolved by node
/// id, making selection independent of discovery order.
pub fn owner<'a>(
    resource: &str,
    nodes: &'a [NodeRecord],
    admitted: &BTreeSet<String>,
) -> Option<&'a NodeRecord> {
    owner_for_operation(resource, nodes, None, admitted)
}

/// Picks a rendezvous-hash owner that advertises `required_operation`.
///
/// The caller can use this for read or execution placement before it forwards
/// a request. `None` retains the ordinary mutable-state ownership behavior.
///
/// Only members of `admitted` are eligible. `node_id` is self-chosen by
/// whoever asks to join, so without this restriction any party could pick an
/// id that hashes to whatever resource it wanted to own — including the
/// `capable_owner` path that decides where inference and Holo execution run.
pub fn owner_for_operation<'a>(
    resource: &str,
    nodes: &'a [NodeRecord],
    required_operation: Option<&str>,
    admitted: &BTreeSet<String>,
) -> Option<&'a NodeRecord> {
    nodes
        .iter()
        .filter(|node| {
            !node.node_id.is_empty()
                && !node.endpoint.is_empty()
                && admitted.contains(&node.node_id)
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

/// A digest of the admitted set. It carries no ordering: a receiver compares it
/// for equality with its own and refuses a mismatch, rather than trying to
/// decide which of two epochs is newer.
pub fn epoch(admitted: &BTreeSet<String>) -> String {
    let mut hasher = blake3::Hasher::new();
    for node_id in admitted {
        hasher.update(node_id.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex()[..16].to_owned()
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

    fn all_admitted(nodes: &[NodeRecord]) -> BTreeSet<String> {
        nodes.iter().map(|node| node.node_id.clone()).collect()
    }

    #[test]
    fn ownership_is_independent_of_discovery_order() {
        let first = vec![node("alpha"), node("bravo"), node("charlie")];
        let mut second = first.clone();
        second.reverse();
        let admitted = all_admitted(&first);
        assert_eq!(
            owner("file:report", &first, &admitted).unwrap().node_id,
            owner("file:report", &second, &admitted).unwrap().node_id
        );
    }

    #[test]
    fn unreachable_members_cannot_own_state() {
        let mut unreachable = node("unreachable");
        unreachable.endpoint.clear();
        let nodes = [unreachable, node("live")];
        let admitted = all_admitted(&nodes);
        assert_eq!(
            owner("history:thread", &nodes, &admitted).unwrap().node_id,
            "live"
        );
    }

    #[test]
    fn resource_key_changes_the_assignment() {
        let nodes = vec![node("alpha"), node("bravo"), node("charlie")];
        let admitted = all_admitted(&nodes);
        let owners = (0..64)
            .map(|index| {
                owner(&format!("file:{index}"), &nodes, &admitted)
                    .unwrap()
                    .node_id
                    .clone()
            })
            .collect::<BTreeSet<_>>();
        assert!(owners.len() > 1, "ownership should spread independent keys");
    }

    #[test]
    fn placement_requires_the_advertised_operation() {
        let mut capable = node("capable");
        capable.operations.push("holo.run".to_owned());
        let nodes = [node("ineligible"), capable];
        let admitted = all_admitted(&nodes);
        let selected = owner_for_operation("holo:sample", &nodes, Some("holo.run"), &admitted)
            .expect("capable node");
        assert_eq!(selected.node_id, "capable");
    }

    // Defect 5: an unadmitted identity cannot be chosen, however its id hashes.
    #[test]
    fn only_admitted_members_can_own_state() {
        let nodes = vec![node("alpha"), node("bravo"), node("charlie")];
        let admitted: BTreeSet<String> = ["bravo".to_owned()].into_iter().collect();
        for index in 0..32 {
            let selected = owner_for_operation(&format!("file:{index}"), &nodes, None, &admitted)
                .expect("an admitted owner");
            assert_eq!(selected.node_id, "bravo");
        }
    }

    #[test]
    fn no_admitted_member_means_no_owner() {
        let nodes = vec![node("alpha")];
        assert!(owner_for_operation("file:x", &nodes, None, &BTreeSet::new()).is_none());
    }

    #[test]
    fn the_epoch_tracks_the_admitted_set_and_not_its_order() {
        let forward: BTreeSet<String> = ["a".to_owned(), "b".to_owned()].into_iter().collect();
        let reverse: BTreeSet<String> = ["b".to_owned(), "a".to_owned()].into_iter().collect();
        assert_eq!(epoch(&forward), epoch(&reverse));

        let grown: BTreeSet<String> = ["a".to_owned(), "b".to_owned(), "c".to_owned()]
            .into_iter()
            .collect();
        assert_ne!(epoch(&forward), epoch(&grown));
    }
}
