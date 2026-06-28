//! OpenAI-compatible Chat Completions provider (DeepSeek, OpenAI, etc.).
//!
//! Stateless: sends system prompt + full message history each turn, parses the
//! SSE `data:` stream and yields `choices[0].delta.content`.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use super::{BoxError, LlmProvider, LlmRequest, LlmStream, Role};
use crate::config::LlmConfig;

pub struct OpenaiChatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f32,
    max_tokens: u32,
    thinking: bool,
}

impl OpenaiChatProvider {
    pub fn new(cfg: &LlmConfig) -> Result<Self, BoxError> {
        if cfg.base_url.is_empty() {
            return Err("openai-chat: LLM_BASE_URL is empty".into());
        }
        if cfg.api_key.is_empty() {
            return Err("openai-chat: LLM_API_KEY is empty".into());
        }
        if cfg.model.is_empty() {
            return Err("openai-chat: LLM_MODEL is empty".into());
        }
        Ok(Self {
            http: reqwest::Client::new(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            thinking: cfg.thinking,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenaiChatProvider {
    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, BoxError> {
        let mut msgs: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
        if !req.system_prompt.is_empty() {
            msgs.push(json!({"role": "system", "content": req.system_prompt}));
        }
        for m in &req.messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            msgs.push(json!({"role": role, "content": m.content}));
        }
        let body = json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
        });
        let mut body = body;
        if !self.thinking {
            // DeepSeek v4: disable reasoning for faster/cheaper replies.
            body["thinking"] = json!({"type": "disabled"});
        }

        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(format!("openai-chat HTTP {st}: {txt}").into());
        }

        let s = async_stream::try_stream! {
            let mut bytes = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = bytes.next().await {
                let chunk = chunk?;
                buf.push_str(&String::from_utf8_lossy(&chunk));
                // SSE: parse whole lines; events are "data: {json}" / "data: [DONE]".
                while let Some(nl) = buf.find('\n') {
                    let line: String = buf[..nl].trim().to_string();
                    buf.drain(..=nl);
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data.is_empty() { continue; }
                    if data == "[DONE]" { return; }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                            if !t.is_empty() {
                                yield t.to_string();
                            }
                        }
                    }
                }
            }
        };
        Ok(Box::pin(s))
    }
}
