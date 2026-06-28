//! Volcengine 语音合成 (TTS) — WebSocket 单向流式 V3. One-shot text in, streamed
//! audio out. Output is raw PCM16 / 16 kHz / mono (audio_params.format = pcm).

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use uuid::Uuid;

use super::super::TextToSpeech;
use super::protocol;

type BoxError = Box<dyn Error + Send + Sync>;

const ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/tts/unidirectional/stream";

pub struct VolcTts {
    api_key: String,
    resource_id: String,
    voice: String,
}

impl VolcTts {
    pub fn new(api_key: String, resource_id: String, voice: String) -> Self {
        Self {
            api_key,
            resource_id,
            voice,
        }
    }

    async fn run(&self, text: &str, voice: &str) -> Result<Vec<u8>, BoxError> {
        let mut req = ENDPOINT.into_client_request()?;
        {
            let h = req.headers_mut();
            h.insert("X-Api-Key", HeaderValue::from_str(&self.api_key)?);
            h.insert("X-Api-Resource-Id", HeaderValue::from_str(&self.resource_id)?);
            h.insert("X-Api-Request-Id", HeaderValue::from_str(&Uuid::new_v4().to_string())?);
        }

        let (ws, _resp) = match tokio_tungstenite::connect_async(req).await {
            Ok(v) => v,
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => {
                let body = r
                    .body()
                    .as_ref()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default();
                return Err(format!("volc tts handshake {} body={}", r.status(), body).into());
            }
            Err(e) => return Err(e.into()),
        };
        let (mut tx, mut rx) = ws.split();

        // SendText (full client request, no event number)
        let payload = serde_json::json!({
            "user": { "uid": "kura-chan" },
            "req_params": {
                "text": text,
                "speaker": if voice.is_empty() { self.voice.as_str() } else { voice },
                "audio_params": { "format": "pcm", "sample_rate": 16000 }
            }
        });
        let body = serde_json::to_vec(&payload)?;
        tx.send(Message::Binary(
            protocol::build(
                protocol::MSG_FULL_CLIENT_REQUEST,
                protocol::FLAG_NONE,
                protocol::SER_JSON,
                &body,
            )
            .into(),
        ))
        .await?;

        // Collect audio from TTSResponse (event 352) frames until SessionFinished (152).
        let mut audio: Vec<u8> = Vec::new();
        let mut logged = false;
        while let Some(msg) = rx.next().await {
            match msg? {
                Message::Binary(b) => {
                    let Some(frame) = protocol::parse_v3(&b) else {
                        continue;
                    };
                    if frame.msg_type == protocol::MSG_SERVER_ERROR {
                        return Err(format!(
                            "volc tts error code={:?} msg={}",
                            frame.error_code,
                            String::from_utf8_lossy(&frame.payload)
                        )
                        .into());
                    }
                    if !logged {
                        logged = true;
                        tracing::debug!(event = ?frame.event, payload_len = frame.payload.len(), "Volc TTS first frame");
                    }
                    match frame.event {
                        Some(protocol::EV_TTS_RESPONSE) => audio.extend_from_slice(&frame.payload),
                        Some(protocol::EV_SESSION_FINISHED) => break,
                        _ => {}
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok(audio)
    }
}

impl TextToSpeech for VolcTts {
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        voice: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, BoxError>> + Send + 'a>> {
        Box::pin(async move {
            match tokio::time::timeout(Duration::from_secs(20), self.run(text, voice)).await {
                Ok(r) => r,
                Err(_) => Err("volc tts timeout".into()),
            }
        })
    }
}
