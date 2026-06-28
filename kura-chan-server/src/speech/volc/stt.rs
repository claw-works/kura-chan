//! Volcengine 大模型流式语音识别 (ASR) — one-shot transcription over the
//! `bigmodel_nostream` endpoint. Input is raw PCM16 / 16 kHz / mono.

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use uuid::Uuid;

use super::super::SpeechToText;
use super::protocol;

type BoxError = Box<dyn Error + Send + Sync>;

const ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream";
// ~200ms of PCM16 @ 16kHz mono = 16000 * 2 * 0.2 = 6400 bytes
const AUDIO_CHUNK: usize = 6400;

pub struct VolcStt {
    api_key: String,
    resource_id: String,
}

impl VolcStt {
    pub fn new(api_key: String, resource_id: String) -> Self {
        Self {
            api_key,
            resource_id,
        }
    }

    async fn run(&self, audio: &[u8]) -> Result<String, BoxError> {
        let mut req = ENDPOINT.into_client_request()?;
        {
            let h = req.headers_mut();
            h.insert("X-Api-Key", HeaderValue::from_str(&self.api_key)?);
            h.insert("X-Api-Resource-Id", HeaderValue::from_str(&self.resource_id)?);
            h.insert("X-Api-Request-Id", HeaderValue::from_str(&Uuid::new_v4().to_string())?);
            h.insert("X-Api-Connect-Id", HeaderValue::from_str(&Uuid::new_v4().to_string())?);
        }

        let (ws, resp) = match tokio_tungstenite::connect_async(req).await {
            Ok(v) => v,
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => {
                let body = r
                    .body()
                    .as_ref()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default();
                return Err(format!("volc asr handshake {} body={}", r.status(), body).into());
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(logid) = resp.headers().get("x-tt-logid") {
            tracing::debug!(logid = ?logid, "Volc ASR connected");
        }
        let (mut tx, mut rx) = ws.split();

        // 1) full client request with audio/request config
        let config = serde_json::json!({
            "user": { "uid": "kura-chan" },
            "audio": { "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 },
            "request": { "model_name": "bigmodel", "enable_punc": true, "enable_itn": true }
        });
        let cfg = serde_json::to_vec(&config)?;
        tx.send(Message::Binary(
            protocol::build(
                protocol::MSG_FULL_CLIENT_REQUEST,
                protocol::FLAG_NONE,
                protocol::SER_JSON,
                &cfg,
            )
            .into(),
        ))
        .await?;

        // 2) audio packets, last one flagged
        if audio.is_empty() {
            tx.send(Message::Binary(
                protocol::build(
                    protocol::MSG_AUDIO_ONLY_REQUEST,
                    protocol::FLAG_LAST_NO_SEQ,
                    protocol::SER_RAW,
                    &[],
                )
                .into(),
            ))
            .await?;
        } else {
            let mut i = 0;
            while i < audio.len() {
                let end = (i + AUDIO_CHUNK).min(audio.len());
                let last = end >= audio.len();
                let flags = if last {
                    protocol::FLAG_LAST_NO_SEQ
                } else {
                    protocol::FLAG_NONE
                };
                tx.send(Message::Binary(
                    protocol::build(
                        protocol::MSG_AUDIO_ONLY_REQUEST,
                        flags,
                        protocol::SER_RAW,
                        &audio[i..end],
                    )
                    .into(),
                ))
                .await?;
                i = end;
            }
        }

        // 3) read server responses, accumulate result text
        let mut text = String::new();
        while let Some(msg) = rx.next().await {
            match msg? {
                Message::Binary(b) => {
                    let Some(frame) = protocol::parse(&b) else {
                        continue;
                    };
                    if frame.msg_type == protocol::MSG_SERVER_ERROR {
                        return Err(format!(
                            "volc asr error code={:?} msg={}",
                            frame.error_code,
                            String::from_utf8_lossy(&frame.payload)
                        )
                        .into());
                    }
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&frame.payload) {
                        if let Some(t) = v
                            .get("result")
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            text = t.to_string();
                        }
                    }
                    if frame.is_last() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        tracing::info!(text = %text, "🎤 ASR recognized");
        Ok(text)
    }
}

impl SpeechToText for VolcStt {
    fn transcribe<'a>(
        &'a self,
        audio: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<String, BoxError>> + Send + 'a>> {
        Box::pin(async move {
            match tokio::time::timeout(Duration::from_secs(15), self.run(audio)).await {
                Ok(r) => r,
                Err(_) => Err("volc asr timeout".into()),
            }
        })
    }
}
