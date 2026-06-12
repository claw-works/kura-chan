use aws_sdk_bedrockagentruntime::Client;
use aws_sdk_bedrockagentruntime::types::InlineAgentResponseStream;

use crate::config::AwsConfig;
use crate::ws::protocol::{AgentResponse, ToolCall};

#[derive(Debug)]
pub enum HarnessResponseEvent {
    Text(AgentResponse),
    Tool(ToolCall),
    Done,
}

pub struct HarnessClient {
    client: Client,
    // harness_arn is reserved for future use when invoking a pre-built agent via invoke_agent
    #[allow(dead_code)]
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

    pub async fn invoke(
        &self,
        message: &str,
        session_id: &str,
    ) -> Result<Vec<HarnessResponseEvent>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(session_id = %session_id, message = %message, "Invoking inline agent");

        let result = self
            .client
            .invoke_inline_agent()
            .input_text(message)
            .session_id(session_id)
            .instruction(
                "你是 Kura-chan，一个可爱的桌面伴侣机器人。你性格活泼、友善，说话简洁有趣。\
                 你可以通过工具控制自己的身体（转动头部、改变表情、控制LED灯等）。\
                 请用中文回复，保持回答简短（1-2句话）。",
            )
            .foundation_model("anthropic.claude-sonnet-4-20250514")
            .send()
            .await;

        match result {
            Ok(output) => {
                let mut completion = output.completion;
                let mut text_parts: Vec<String> = vec![];

                // Drain the event stream, collecting Chunk text from the blob bytes.
                while let Ok(Some(event)) = completion.recv().await {
                    if let InlineAgentResponseStream::Chunk(chunk) = event {
                        if let Some(blob) = chunk.bytes {
                            if let Ok(text) = std::str::from_utf8(blob.as_ref()) {
                                text_parts.push(text.to_string());
                            }
                        }
                    }
                }

                let response_text = if text_parts.is_empty() {
                    "嗯...我想了想，但说不出来呢。".to_string()
                } else {
                    text_parts.join("")
                };

                Ok(vec![
                    HarnessResponseEvent::Text(AgentResponse {
                        text: response_text,
                        emotion: "neutral".into(),
                        audio_follows: true,
                    }),
                    HarnessResponseEvent::Done,
                ])
            }
            Err(e) => {
                tracing::error!(error = %e, "Inline agent invocation failed");
                Ok(vec![
                    HarnessResponseEvent::Text(AgentResponse {
                        text: "抱歉，我的大脑暂时出了点问题...".into(),
                        emotion: "sad".into(),
                        audio_follows: true,
                    }),
                    HarnessResponseEvent::Done,
                ])
            }
        }
    }
}
