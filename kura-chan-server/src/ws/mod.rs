pub mod codec;
pub mod protocol;
pub mod session;

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};

use crate::auth::validate_api_key;
use crate::config::Config;
use crate::error::AppError;
use crate::harness::HarnessClient;
use crate::harness::invoke::HarnessResponseEvent;
use crate::speech::{SpeechToText, TextToSpeech};
use crate::ws::codec::{AudioFrame, AUDIO_OUTPUT, FLAG_END, FLAG_START};
use crate::ws::protocol::*;
use crate::ws::session::{Session, SessionEvent, SessionState};

pub struct AppState {
    pub config: Arc<Config>,
    pub harness: HarnessClient,
    pub stt: Box<dyn SpeechToText>,
    pub tts: Box<dyn TextToSpeech>,
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let device_id = validate_api_key(&headers, &state.config.auth)
        .map_err(AppError::Auth)?;

    tracing::info!(device_id = %device_id, "WebSocket upgrade accepted");

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, device_id, state)))
}

async fn handle_socket(socket: WebSocket, device_id: String, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut session = Session::new(device_id.clone(), state.config.clone());

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(device_id = %device_id, error = %e, "WebSocket read error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let events = match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(ClientMessage::Hello(hello)) => session.handle_hello(hello),
                    Ok(ClientMessage::ToolResult(result)) => session.handle_tool_result(result),
                    Err(e) => {
                        tracing::warn!(error = %e, "Invalid JSON message");
                        vec![SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                            code: "parse_error".into(),
                            message: e.to_string(),
                        }))]
                    }
                };
                for event in events {
                    send_event(&mut sender, event).await;
                }
            }
            Message::Binary(data) => {
                let frame_events = match AudioFrame::decode(&data) {
                    Some(frame) => session.handle_audio_frame(frame),
                    None => vec![SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                        code: "invalid_frame".into(),
                        message: "Could not decode audio frame".into(),
                    }))],
                };

                for event in frame_events {
                    send_event(&mut sender, event).await;
                }

                // If session transitioned to Thinking, run the full pipeline
                if session.state == SessionState::Thinking {
                    let audio_data = std::mem::take(&mut session.audio_buffer);

                    // STT
                    let stt_text: String = state.stt.transcribe(&audio_data).await.unwrap_or_default();
                    send_event(
                        &mut sender,
                        SessionEvent::SendJson(ServerMessage::Stt(SttResult {
                            text: stt_text.clone(),
                            r#final: true,
                        })),
                    )
                    .await;

                    // Invoke harness
                    let harness_events = state
                        .harness
                        .invoke(&stt_text, &session.session_id)
                        .await
                        .unwrap_or_default();

                    for event in harness_events {
                        match event {
                            HarnessResponseEvent::Text(response) => {
                                let tts_audio: Vec<u8> = state
                                    .tts
                                    .synthesize(&response.text)
                                    .await
                                    .unwrap_or_default();

                                let speak_events = session.transition_to_speaking(response);
                                for ev in speak_events {
                                    send_event(&mut sender, ev).await;
                                }

                                if !tts_audio.is_empty() {
                                    let out_frame = AudioFrame {
                                        frame_type: AUDIO_OUTPUT,
                                        flags: FLAG_START | FLAG_END,
                                        payload: tts_audio,
                                    };
                                    send_event(
                                        &mut sender,
                                        SessionEvent::SendAudio(out_frame.encode()),
                                    )
                                    .await;
                                }
                            }
                            HarnessResponseEvent::Tool(tool_call) => {
                                send_event(
                                    &mut sender,
                                    SessionEvent::SendJson(ServerMessage::ToolCall(tool_call)),
                                )
                                .await;
                            }
                            HarnessResponseEvent::Done => {
                                let done_events = session.finish_speaking();
                                for ev in done_events {
                                    send_event(&mut sender, ev).await;
                                }
                            }
                        }
                    }
                }
            }
            Message::Ping(data) => {
                let _ = sender.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    tracing::info!(device_id = %device_id, "Device disconnected");
}

async fn send_event(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: SessionEvent,
) {
    match event {
        SessionEvent::SendJson(msg) => {
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(Message::Text(json.into())).await;
            }
        }
        SessionEvent::SendAudio(data) => {
            let _ = sender.send(Message::Binary(data.into())).await;
        }
    }
}
