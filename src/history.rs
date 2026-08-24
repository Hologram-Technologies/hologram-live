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
            archived: false,
        };
        self.persist(&conversation)?;
        Ok(conversation)
    }

    pub fn list(&self, include_archived: bool) -> Result<Vec<Conversation>> {
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
            let conversation = serde_json::from_slice::<Conversation>(&bytes)?;
            if conversation.archived && !include_archived {
                continue;
            }
            output.push(conversation);
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

    /// Records one user turn and its assistant response in a single write.
    pub fn append_exchange(
        &self,
        id: &str,
        user_content: String,
        assistant_content: String,
    ) -> Result<Conversation> {
        if user_content.trim().is_empty() {
            return Err(LiveError::Config("message content is empty".to_owned()));
        }
        if assistant_content.trim().is_empty() {
            return Err(LiveError::Config("assistant response is empty".to_owned()));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("history lock poisoned".to_owned()))?;
        let mut conversation = self.get(id)?;
        let user_created_at_millis = now_millis();
        conversation.messages.push(ConversationMessage {
            role: "user".to_owned(),
            content: user_content,
            created_at_millis: user_created_at_millis,
        });
        let assistant_created_at_millis = now_millis().max(user_created_at_millis);
        conversation.messages.push(ConversationMessage {
            role: "assistant".to_owned(),
            content: assistant_content,
            created_at_millis: assistant_created_at_millis,
        });
        conversation.updated_at_millis = assistant_created_at_millis;
        self.persist_unlocked(&conversation)?;
        Ok(conversation)
    }

    /// Archiving hides a conversation from the default listing without deleting it.
    /// The timestamp is left alone so archiving does not reorder the list.
    pub fn set_archived(&self, id: &str, archived: bool) -> Result<Conversation> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("history lock poisoned".to_owned()))?;
        let mut conversation = self.get(id)?;
        conversation.archived = archived;
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

    #[test]
    fn archiving_hides_a_conversation_from_the_default_listing() {
        let root = std::env::temp_dir().join(format!("hologram-archive-{}", now_millis()));
        let history = HistoryService::open(&root).expect("open");
        let kept = history.create("kept".to_owned()).expect("create kept");
        let hidden = history.create("hidden".to_owned()).expect("create hidden");

        let archived = history.set_archived(&hidden.id, true).expect("archive");
        assert!(archived.archived);
        assert_eq!(
            archived.updated_at_millis, hidden.updated_at_millis,
            "archiving must not reorder the listing"
        );

        let visible = history.list(false).expect("list visible");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, kept.id);

        let all = history.list(true).expect("list all");
        assert_eq!(all.len(), 2);

        history.set_archived(&hidden.id, false).expect("unarchive");
        assert_eq!(history.list(false).expect("list restored").len(), 2);
        assert!(!history.get(&hidden.id).expect("get").archived);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn conversations_written_before_archiving_still_load() {
        let root = std::env::temp_dir().join(format!("hologram-legacy-{}", now_millis()));
        let history = HistoryService::open(&root).expect("open");
        let conversation = history.create("legacy".to_owned()).expect("create");

        // Rewrite the record without the `archived` field, as older builds stored it.
        let path = history.path_for(&conversation.id).expect("path");
        let legacy = serde_json::json!({
            "id": conversation.id,
            "title": conversation.title,
            "created_at_millis": conversation.created_at_millis,
            "updated_at_millis": conversation.updated_at_millis,
            "messages": [],
        });
        std::fs::write(&path, serde_json::to_vec(&legacy).expect("encode")).expect("write");

        let loaded = history.get(&conversation.id).expect("get legacy");
        assert!(!loaded.archived);
        assert_eq!(history.list(false).expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exchanges_persist_both_sides_of_a_chat_turn() {
        let root = std::env::temp_dir().join(format!("hologram-chat-history-{}", now_millis()));
        let history = HistoryService::open(&root).expect("open");
        let conversation = history.create("echo".to_owned()).expect("create");
        let conversation = history
            .append_exchange(&conversation.id, "hello".to_owned(), "hello".to_owned())
            .expect("append exchange");
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.messages[0].role, "user");
        assert_eq!(conversation.messages[1].role, "assistant");
        assert_eq!(conversation.messages[1].content, "hello");
        let second = history
            .create("another thread".to_owned())
            .expect("create second");
        history
            .append_exchange(&second.id, "goodbye".to_owned(), "goodbye".to_owned())
            .expect("append second exchange");
        assert_eq!(
            history.get(&conversation.id).expect("get first").messages[0].content,
            "hello"
        );
        assert_eq!(
            history.get(&second.id).expect("get second").messages[0].content,
            "goodbye"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
