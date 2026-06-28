//! Bedrock AgentCore harness provider — wraps the existing `HarnessClient`.
//!
//! Harness keeps conversation history server-side (keyed by `session_id`), so we
//! send only the current user message; `messages` history is ignored here.

use async_trait::async_trait;

use super::{BoxError, LlmProvider, LlmRequest, LlmStream, Role};
use crate::config::AwsConfig;
use crate::harness::HarnessClient;
use crate::harness::invoke::extract_text_delta;

pub struct BedrockHarnessProvider {
    client: HarnessClient,
}

impl BedrockHarnessProvider {
    pub async fn new(aws: &AwsConfig) -> Self {
        Self { client: HarnessClient::new(aws).await }
    }
}

#[async_trait]
impl LlmProvider for BedrockHarnessProvider {
    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, BoxError> {
        // Current user turn = last user message in the list.
        let current = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let out = self
            .client
            .invoke_stream(&current, &req.session_id, &req.actor_id, &req.system_prompt)
            .await?;
        let s = async_stream::stream! {
            let mut stream = out.stream;
            loop {
                match stream.recv().await {
                    Ok(Some(event)) => {
                        if let Some(t) = extract_text_delta(&event) {
                            yield Ok(t);
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        yield Err(Box::new(e) as BoxError);
                        break;
                    }
                }
            }
        };
        Ok(Box::pin(s))
    }
}
