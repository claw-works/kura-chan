use aws_sdk_bedrockagentcore::Client;
use aws_sdk_bedrockagentcore::operation::invoke_harness::InvokeHarnessOutput;
use aws_sdk_bedrockagentcore::types::{
    HarnessContentBlock, HarnessConversationRole, HarnessMessage, HarnessSystemContentBlock,
    InvokeHarnessStreamOutput,
};

use crate::config::AwsConfig;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct HarnessClient {
    client: Client,
    harness_arn: String,
}

impl HarnessClient {
    pub async fn new(aws_config: &AwsConfig) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(aws_config.region.clone()))
            .load()
            .await;
        let client = Client::new(&sdk_config);
        Self {
            client,
            harness_arn: aws_config.harness_arn.clone(),
        }
    }

    /// Start a harness invocation. `actor_id` ties long-term memory; `session_id`
    /// is the conversation thread; `system_prompt` is the (per-actor) persona+rules.
    pub async fn invoke_stream(
        &self,
        message: &str,
        session_id: &str,
        actor_id: &str,
        system_prompt: &str,
    ) -> Result<InvokeHarnessOutput, BoxError> {
        tracing::info!(actor_id = %actor_id, session_id = %session_id, message = %message, "Invoking harness (stream)");
        let user_message = HarnessMessage::builder()
            .role(HarnessConversationRole::User)
            .content(HarnessContentBlock::Text(message.to_string()))
            .build()?;
        let out = self
            .client
            .invoke_harness()
            .harness_arn(&self.harness_arn)
            .runtime_session_id(session_id)
            .runtime_user_id(actor_id)
            .actor_id(actor_id)
            .system_prompt(HarnessSystemContentBlock::Text(system_prompt.to_string()))
            .messages(user_message)
            .send()
            .await?;
        Ok(out)
    }
}

/// Pull a text fragment out of a ContentBlockDelta stream event, if present.
pub fn extract_text_delta(event: &InvokeHarnessStreamOutput) -> Option<String> {
    let delta_event = event.as_content_block_delta().ok()?;
    let delta = delta_event.delta()?;
    delta.as_text().ok().map(|t| t.to_string())
}
