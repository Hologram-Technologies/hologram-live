//! Local fallback engine: the assistant response repeats the user's message.

use super::{Completion, CompletionRequest, InferenceEngine};
use crate::error::Result;
use crate::models::ModelInfo;
use std::time::Instant;

pub struct EchoEngine;

#[tonic::async_trait]
impl InferenceEngine for EchoEngine {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let started = Instant::now();
        Ok(Completion {
            text: last_user_content(&request.prompt).to_owned(),
            model: "echo".to_owned(),
            tokens_per_second: None,
            elapsed_millis: super::elapsed_millis(started),
            usage: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }
}

/// Newest user turn of a rendered `role: content` transcript. Prompts without
/// transcript framing are returned unchanged, preserving the original
/// echo-demo behavior for a bare single-turn prompt.
fn last_user_content(prompt: &str) -> &str {
    for line in prompt.lines().rev() {
        if let Some(content) = line.strip_prefix("user: ") {
            return content;
        }
    }
    prompt.trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: text.to_owned(),
            ..CompletionRequest::default()
        }
    }

    #[tokio::test]
    async fn echo_returns_the_newest_user_turn() {
        let engine = EchoEngine;
        let completion = engine
            .complete(prompt("user: first\nassistant: first\nuser: second"))
            .await
            .expect("echo");
        assert_eq!(completion.text, "second");
        assert_eq!(engine.name(), "echo");
        assert!(engine.list_models().await.expect("models").is_empty());
    }

    #[tokio::test]
    async fn echo_returns_a_bare_prompt_unchanged() {
        let engine = EchoEngine;
        let completion = engine.complete(prompt("hello")).await.expect("echo");
        assert_eq!(completion.text, "hello");
    }
}
