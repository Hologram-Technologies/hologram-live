//! Conversation-backed chat over the configured inference engine.

use crate::error::{LiveError, Result};
use crate::history::HistoryService;
use crate::inference::{CompletionRequest, InferenceEngine};
use crate::protocol::Conversation;
use std::sync::Arc;

pub struct ChatService {
    history: Arc<HistoryService>,
    engine: Arc<dyn InferenceEngine>,
}

impl ChatService {
    pub fn new(history: Arc<HistoryService>, engine: Arc<dyn InferenceEngine>) -> Self {
        Self { history, engine }
    }

    pub fn engine_name(&self) -> &'static str {
        self.engine.name()
    }

    /// The configured engine, shared with the compatibility HTTP modules.
    pub fn engine(&self) -> &Arc<dyn InferenceEngine> {
        &self.engine
    }

    /// Appends the user message and the engine's response as one exchange.
    pub async fn send(&self, conversation_id: &str, content: String) -> Result<Conversation> {
        if self.engine.name() == "echo" {
            // Short-circuit: preserve the original echo-demo behavior exactly.
            let history = self.history.clone();
            let id = conversation_id.to_owned();
            let echoed = content.clone();
            return spawn_history(move || history.append_exchange(&id, content, echoed)).await;
        }
        let history = self.history.clone();
        let id = conversation_id.to_owned();
        let request = if self.engine.supports_sessions() {
            // The engine's resident session holds the context, so only the
            // raw new turn is sent — no transcript rendering.
            CompletionRequest {
                prompt: content.clone(),
                session_key: Some(conversation_id.to_owned()),
                ..CompletionRequest::default()
            }
        } else {
            let history = self.history.clone();
            let id = conversation_id.to_owned();
            let conversation = spawn_history(move || history.get(&id)).await?;
            CompletionRequest {
                prompt: render_transcript(&conversation, &content),
                ..CompletionRequest::default()
            }
        };
        let completion = self.engine.complete(request).await?;
        spawn_history(move || history.append_exchange(&id, content, completion.text)).await
    }
}

/// Renders history plus the new user turn as plain `role: content` lines.
/// Engines apply their own chat templates (weightc artifacts carry theirs),
/// so daemon-side rendering stays minimal.
pub fn render_transcript(conversation: &Conversation, new_user_content: &str) -> String {
    let mut lines = Vec::with_capacity(conversation.messages.len() + 1);
    for message in &conversation.messages {
        lines.push(format!("{}: {}", message.role, message.content));
    }
    lines.push(format!("user: {new_user_content}"));
    lines.join("\n")
}

async fn spawn_history<T>(function: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(function)
        .await
        .map_err(|error| LiveError::Conflict(format!("join chat history: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::Completion;
    use crate::models::ModelInfo;

    struct Fixture {
        _temporary: tempfile::TempDir,
        history: Arc<HistoryService>,
    }

    fn fixture() -> Fixture {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let history =
            Arc::new(HistoryService::open(temporary.path().join("history")).expect("history"));
        Fixture {
            history,
            _temporary: temporary,
        }
    }

    #[test]
    fn transcript_renders_role_prefixed_lines_with_the_new_turn_last() {
        let fixture = fixture();
        let conversation = fixture.history.create("demo".to_owned()).expect("create");
        let conversation = fixture
            .history
            .append_exchange(&conversation.id, "hi".to_owned(), "hello".to_owned())
            .expect("exchange");

        let transcript = render_transcript(&conversation, "how are you");

        assert_eq!(transcript, "user: hi\nassistant: hello\nuser: how are you");
    }

    #[tokio::test]
    async fn echo_engine_matches_the_original_demo_behavior() {
        let fixture = fixture();
        let conversation = fixture.history.create("demo".to_owned()).expect("create");
        let chat = ChatService::new(
            fixture.history.clone(),
            Arc::new(crate::inference::EchoEngine),
        );

        let updated = chat
            .send(&conversation.id, "echo me".to_owned())
            .await
            .expect("send");

        assert_eq!(updated.messages.len(), 2);
        assert_eq!(updated.messages[0].role, "user");
        assert_eq!(updated.messages[0].content, "echo me");
        assert_eq!(updated.messages[1].role, "assistant");
        assert_eq!(updated.messages[1].content, "echo me");
    }

    struct StubEngine {
        reply: String,
    }

    #[tonic::async_trait]
    impl InferenceEngine for StubEngine {
        fn name(&self) -> &'static str {
            "stub"
        }

        async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
            assert!(request.prompt.ends_with("user: ping"));
            Ok(Completion {
                text: self.reply.clone(),
                model: "stub".to_owned(),
                tokens_per_second: None,
                elapsed_millis: 0,
            })
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn non_echo_engines_receive_the_transcript_and_reply_is_recorded() {
        let fixture = fixture();
        let conversation = fixture.history.create("demo".to_owned()).expect("create");
        let chat = ChatService::new(
            fixture.history.clone(),
            Arc::new(StubEngine {
                reply: "pong".to_owned(),
            }),
        );

        let updated = chat
            .send(&conversation.id, "ping".to_owned())
            .await
            .expect("send");

        assert_eq!(updated.messages[1].content, "pong");
    }

    struct SessionStubEngine {
        seen: std::sync::Mutex<Vec<CompletionRequest>>,
    }

    #[tonic::async_trait]
    impl InferenceEngine for SessionStubEngine {
        fn name(&self) -> &'static str {
            "session-stub"
        }

        fn supports_sessions(&self) -> bool {
            true
        }

        async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
            self.seen.lock().expect("seen").push(request);
            Ok(Completion {
                text: "reply".to_owned(),
                model: "session-stub".to_owned(),
                tokens_per_second: None,
                elapsed_millis: 0,
            })
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn session_capable_engines_receive_the_raw_turn_and_conversation_key() {
        let fixture = fixture();
        let conversation = fixture.history.create("demo".to_owned()).expect("create");
        let engine = Arc::new(SessionStubEngine {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let chat = ChatService::new(fixture.history.clone(), engine.clone());

        chat.send(&conversation.id, "first turn".to_owned())
            .await
            .expect("first send");
        chat.send(&conversation.id, "second turn".to_owned())
            .await
            .expect("second send");

        let seen = engine.seen.lock().expect("seen");
        assert_eq!(seen.len(), 2);
        // No transcript rendering: the resident session holds the context.
        assert_eq!(seen[0].prompt, "first turn");
        assert_eq!(seen[1].prompt, "second turn");
        assert_eq!(
            seen[0].session_key.as_deref(),
            Some(conversation.id.as_str())
        );
        assert_eq!(seen[1].session_key, seen[0].session_key);
    }
}
