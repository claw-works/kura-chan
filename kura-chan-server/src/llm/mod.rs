//! LLM provider abstraction.
//!
//! One `LlmProvider` trait, multiple API-format backends selected via config
//! (`[llm].format` / `LLM_FORMAT`). Stateless providers (openai/anthropic) get
//! the full conversation history in `messages`; server-side-session providers
//! (bedrock-harness) keep history themselves via `session_id`.

pub mod bedrock_harness;
pub mod openai_chat;

use std::pin::Pin;
use std::str::FromStr;

use async_trait::async_trait;
use futures_util::Stream;

use crate::config::Config;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A stream of assistant text deltas.
pub type LlmStream = Pin<Box<dyn Stream<Item = Result<String, BoxError>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// One LLM turn. `messages` is the ordered history ending with the current user
/// message. `session_id`/`actor_id` are used by session-stateful providers.
pub struct LlmRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub session_id: String,
    pub actor_id: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stream the assistant reply as text deltas.
    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, BoxError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFormat {
    BedrockHarness,
    BedrockConverse,
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
}

impl FromStr for LlmFormat {
    type Err = BoxError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bedrock-harness" => Ok(Self::BedrockHarness),
            "bedrock-converse" => Ok(Self::BedrockConverse),
            "openai-chat" => Ok(Self::OpenaiChat),
            "openai-responses" => Ok(Self::OpenaiResponses),
            "anthropic-messages" => Ok(Self::AnthropicMessages),
            other => Err(format!("unknown LLM format '{other}'").into()),
        }
    }
}

/// Build the configured provider. Only `bedrock-harness` and `openai-chat` are
/// implemented today; the rest are reserved for later.
pub async fn build_provider(config: &Config) -> Result<Box<dyn LlmProvider>, BoxError> {
    let fmt: LlmFormat = config.llm.format.parse()?;
    tracing::info!(
        format = %config.llm.format,
        model = %config.llm.model,
        thinking = config.llm.thinking,
        max_tokens = config.llm.max_tokens,
        temperature = config.llm.temperature,
        "LLM provider"
    );
    match fmt {
        LlmFormat::BedrockHarness => {
            Ok(Box::new(bedrock_harness::BedrockHarnessProvider::new(&config.aws).await))
        }
        LlmFormat::OpenaiChat => {
            Ok(Box::new(openai_chat::OpenaiChatProvider::new(&config.llm)?))
        }
        other => Err(format!("LLM format {other:?} not implemented yet").into()),
    }
}
