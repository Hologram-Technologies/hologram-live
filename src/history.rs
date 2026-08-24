use crate::error::{LiveError, Result};
use crate::protocol::{Conversation, ConversationMessage};
use crate::util::{atomic_write, now_millis};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct HistoryService {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl HistoryService {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|error| LiveError::io(&root, error))?;
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    pub fn create(&self, title: String) -> Result<Conversation> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(LiveError::Config("conversation title is empty".to_owned()));
        }
        let created_at_millis = now_millis();
        let nonce = format!("{title}\0{created_at_millis}\0{}", std::process::id());
        let id = format!("blake3:{}", blake3::hash(nonce.as_bytes()).to_hex());
        let conversation = Conversation {
            id,
            title,
            created_at_millis,
            updated_at_millis: created_at_millis,
            messages: Vec::new(),
        };
        self.persist(&conversation)?;
        Ok(conversation)
    }

    pub fn list(&self) -> Result<Vec<Conversation>> {
        let mut output = Vec::new();
        for entry in
            std::fs::read_dir(&self.root).map_err(|error| LiveError::io(&self.root, error))?
        {
            let entry = entry.map_err(|error| LiveError::io(&self.root, error))?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes =
                std::fs::read(entry.path()).map_err(|error| LiveError::io(&entry.path(), error))?;
            output.push(serde_json::from_slice::<Conversation>(&bytes)?);
        }
        output.sort_by_key(|item| std::cmp::Reverse(item.updated_at_millis));
        Ok(output)
    }

    pub fn get(&self, id: &str) -> Result<Conversation> {
        let path = self.path_for(id)?;
        let bytes = std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::NotFound(format!("conversation {id} not found"))
            } else {
                LiveError::io(&path, error)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(Into::into)
    }

    pub fn append(&self, id: &str, role: String, content: String) -> Result<Conversation> {
        validate_role(&role)?;
        if content.trim().is_empty() {
            return Err(LiveError::Config("message content is empty".to_owned()));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("history lock poisoned".to_owned()))?;
        let mut conversation = self.get(id)?;
        let now = now_millis();
        conversation.messages.push(ConversationMessage {
            role,
            content,
            created_at_millis: now,
        });
        conversation.updated_at_millis = now;
        self.persist_unlocked(&conversation)?;
        Ok(conversation)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id)?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| LiveError::io(&path, error))?;
        }
        Ok(())
    }

    fn persist(&self, conversation: &Conversation) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("history lock poisoned".to_owned()))?;
        self.persist_unlocked(conversation)
    }

    fn persist_unlocked(&self, conversation: &Conversation) -> Result<()> {
        let path = self.path_for(&conversation.id)?;
        let bytes = serde_json::to_vec_pretty(conversation)?;
        atomic_write(&path, &bytes)
    }

    fn path_for(&self, id: &str) -> Result<PathBuf> {
        let digest = id.strip_prefix("blake3:").ok_or_else(|| {
            LiveError::NotFound("conversation id must be a BLAKE3 address".to_owned())
        })?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(LiveError::NotFound("malformed conversation id".to_owned()));
        }
        Ok(self.root.join(format!("{digest}.json")))
    }
}

fn validate_role(role: &str) -> Result<()> {
    match role {
        "system" | "user" | "assistant" | "tool" => Ok(()),
        _ => Err(LiveError::Config(format!(
            "unsupported role {role:?}; expected system, user, assistant, or tool"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversations_persist() {
        let root = std::env::temp_dir().join(format!("hologram-history-{}", now_millis()));
        let history = HistoryService::open(&root).expect("open");
        let conversation = history.create("demo".to_owned()).expect("create");
        history
            .append(&conversation.id, "user".to_owned(), "hello".to_owned())
            .expect("append");
        assert_eq!(
            history.get(&conversation.id).expect("get").messages.len(),
            1
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
