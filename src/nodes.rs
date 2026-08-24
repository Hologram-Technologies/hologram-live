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
}
