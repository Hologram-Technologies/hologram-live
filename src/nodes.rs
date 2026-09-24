use crate::error::{LiveError, Result};
use crate::protocol::NodeRecord;
use crate::util::{atomic_write, now_millis};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct NodeDirectory {
    path: PathBuf,
    nodes: Mutex<BTreeMap<String, NodeRecord>>,
}

impl NodeDirectory {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let nodes = if path.exists() {
            let bytes = std::fs::read(&path).map_err(|error| LiveError::io(&path, error))?;
            serde_json::from_slice(&bytes)?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path,
            nodes: Mutex::new(nodes),
        })
    }

    pub fn heartbeat(&self, mut node: NodeRecord) -> Result<()> {
        node.last_seen_millis = now_millis();
        let mut nodes = self
            .nodes
            .lock()
            .map_err(|_| LiveError::Conflict("node directory lock poisoned".to_owned()))?;
        nodes.insert(node.node_id.clone(), node);
        let bytes = serde_json::to_vec_pretty(&*nodes)?;
        atomic_write(&self.path, &bytes)
    }

    pub fn list(&self) -> Result<Vec<NodeRecord>> {
        let nodes = self
            .nodes
            .lock()
            .map_err(|_| LiveError::Conflict("node directory lock poisoned".to_owned()))?;
        Ok(nodes.values().cloned().collect())
    }

    pub fn prune_older_than(&self, cutoff_millis: u64, preserve_node_id: &str) -> Result<usize> {
        let mut nodes = self
            .nodes
            .lock()
            .map_err(|_| LiveError::Conflict("node directory lock poisoned".to_owned()))?;
        let before = nodes.len();
        nodes.retain(|node_id, node| {
            node_id == preserve_node_id || node.last_seen_millis >= cutoff_millis
        });
        let removed = before - nodes.len();
        if removed > 0 {
            let bytes = serde_json::to_vec_pretty(&*nodes)?;
            atomic_write(&self.path, &bytes)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, last_seen_millis: u64) -> NodeRecord {
        NodeRecord {
            node_id: id.to_owned(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: format!("https://{id}.example"),
            last_seen_millis,
        }
    }

    #[test]
    fn pruning_removes_stale_nodes_but_preserves_self() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nodes = NodeDirectory::open(directory.path().join("nodes.json")).expect("directory");
        nodes.heartbeat(node("self", 0)).expect("self heartbeat");
        nodes.heartbeat(node("peer", 0)).expect("peer heartbeat");

        let removed = nodes
            .prune_older_than(u64::MAX, "self")
            .expect("prune stale nodes");

        assert_eq!(removed, 1);
        assert_eq!(nodes.list().expect("list")[0].node_id, "self");
    }
}
