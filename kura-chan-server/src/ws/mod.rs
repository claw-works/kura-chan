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
use crate::harness::invoke::extract_text_delta;
use crate::speech::{SpeechToText, TextToSpeech};
use crate::ws::codec::{AudioFrame, AUDIO_OUTPUT, FLAG_START};
use crate::ws::protocol::*;
use crate::ws::session::{Session, SessionEvent, SessionState};

pub struct AppState {
    pub config: Arc<Config>,
    pub harness: HarnessClient,
    pub stt: Box<dyn SpeechToText>,
    pub tts: Box<dyn TextToSpeech>,
    /// Cache of synthesized canned phrases (phrase text -> PCM), so common
    /// replies like "没听清" are only synthesized once.
    pub canned: tokio::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

/// Phrase played when speech couldn't be transcribed.
pub const PHRASE_NOT_HEARD: &str = "诶？小爪没听清，再说一遍好不好？";
/// Phrase played when the agent call fails.
pub const PHRASE_ERROR: &str = "呜，小爪的脑袋有点卡住了，等会儿再试试嘛。";

impl AppState {
    /// Get cached PCM for a canned phrase, synthesizing+caching on first use.
    pub async fn canned_audio(&self, text: &str) -> Vec<u8> {
        if let Some(a) = self.canned.lock().await.get(text) {
            return a.clone();
        }
        let audio = self.tts.synthesize(text).await.unwrap_or_default();
        if !audio.is_empty() {
            self.canned.lock().await.insert(text.to_string(), audio.clone());
        }
        audio
    }
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    // Try auth, fallback to "unknown" if headers missing (for debugging)
    let device_id = validate_api_key(&headers, &state.config.auth)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Auth failed, allowing anyway for debug");
            "unknown_device".to_string()
        });

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
                    Ok(ClientMessage::Status(status)) => session.handle_status(status),
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
                    let stt_text: String = match state.stt.transcribe(&audio_data).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::error!(error = %e, "STT failed");
                            String::new()
                        }
                    };
                    send_event(
                        &mut sender,
                        SessionEvent::SendJson(ServerMessage::Stt(SttResult {
                            text: stt_text.clone(),
                            r#final: true,
                        })),
                    )
                    .await;

                    // Couldn't transcribe (silence/noise): play a cached canned phrase.
                    if stt_text.trim().is_empty() {
                        tracing::info!("Empty STT, playing canned phrase");
                        let audio = state.canned_audio(PHRASE_NOT_HEARD).await;
                        speak_phrase(&mut sender, &mut session, PHRASE_NOT_HEARD, "confused", &audio).await;
                        continue;
                    }

                    // Stream the harness reply: synthesize + send sentence by sentence.
                    let user_message = session.build_user_message(&stt_text);
                    let mut output = match state
                        .harness
                        .invoke_stream(&user_message, &session.session_id)
                        .await
                    {
                        Ok(o) => o,
                        Err(e) => {
                            tracing::error!(error = ?e, "Harness invoke failed");
                            let audio = state.canned_audio(PHRASE_ERROR).await;
                            speak_phrase(&mut sender, &mut session, PHRASE_ERROR, "sad", &audio).await;
                            continue;
                        }
                    };

                    for ev in session.transition_to_speaking(AgentResponse {
                        text: String::new(),
                        emotion: "happy".into(),
                        audio_follows: true,
                    }) {
                        send_event(&mut sender, ev).await;
                    }

                    let mut buf = String::new();
                    let mut first = true;
                    let mut sent_any = false;
                    loop {
                        match output.stream.recv().await {
                            Ok(Some(event)) => {
                                if let Some(t) = extract_text_delta(&event) {
                                    buf.push_str(&t);
                                    for msg in extract_tags(&mut buf) {
                                        send_event(&mut sender, SessionEvent::SendJson(msg)).await;
                                    }
                                    while let Some(cut) = split_sentence(&buf) {
                                        let seg: String = buf.drain(..cut).collect();
                                        let seg = seg.trim();
                                        if !seg.is_empty() && !seg.contains("[NOISE]") {
                                            let audio =
                                                state.tts.synthesize(seg).await.unwrap_or_default();
                                            if !audio.is_empty() {
                                                send_audio_stream(&mut sender, &audio, first).await;
                                                first = false;
                                                sent_any = true;
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                tracing::error!(error = %e, "Harness stream error");
                                break;
                            }
                        }
                    }
                    // flush trailing text
                    for msg in extract_tags(&mut buf) {
                        send_event(&mut sender, SessionEvent::SendJson(msg)).await;
                    }
                    let rest = buf.trim().to_string();
                    if !rest.is_empty() && !rest.contains("[NOISE]") {
                        let audio = state.tts.synthesize(&rest).await.unwrap_or_default();
                        if !audio.is_empty() {
                            send_audio_stream(&mut sender, &audio, first).await;
                            first = false;
                            sent_any = true;
                        }
                    }
                    // nothing intelligible → canned "not heard"
                    if !sent_any {
                        tracing::info!("No speakable reply, using canned phrase");
                        let audio = state.canned_audio(PHRASE_NOT_HEARD).await;
                        send_audio_stream(&mut sender, &audio, first).await;
                    }
                    send_event(&mut sender, SessionEvent::SendJson(ServerMessage::SpeakDone))
                        .await;
                    for ev in session.finish_speaking() {
                        send_event(&mut sender, ev).await;
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

/// Push PCM as AUDIO_OUTPUT frames (chunked). `start` marks the very first frame
/// of a reply (device resets its playback buffer and switches to the speaker).
/// No END flag is used; the reply ends with a SpeakDone control message.
async fn send_audio_stream(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    audio: &[u8],
    start: bool,
) {
    const CHUNK: usize = 4096;
    let total = audio.len();
    let mut off = 0;
    let mut first = start;
    while off < total {
        let end = (off + CHUNK).min(total);
        let flags = if first { FLAG_START } else { 0 };
        first = false;
        let frame = AudioFrame {
            frame_type: AUDIO_OUTPUT,
            flags,
            payload: audio[off..end].to_vec(),
        };
        send_event(sender, SessionEvent::SendAudio(frame.encode())).await;
        off = end;
    }
}

/// Remove complete `[do:...]` / `[mood:...]` tags from `buf`, returning the
/// control/emotion messages to send. Unrecognized tags (e.g. `[NOISE]`) and any
/// unclosed trailing `[...` are left in place so TTS handling stays correct.
fn extract_tags(buf: &mut String) -> Vec<ServerMessage> {
    let mut out = Vec::new();
    let mut search = 0;
    loop {
        let Some(rel) = buf[search..].find('[') else { break };
        let open = search + rel;
        let Some(crel) = buf[open..].find(']') else { break }; // unclosed: wait for more
        let close = open + crel;
        let inner = buf[open + 1..close].trim().to_string();
        if let Some(rest) = inner.strip_prefix("do:") {
            if let Some(msg) = parse_do(rest) {
                out.push(msg);
            }
            buf.replace_range(open..close + 1, "");
            search = open;
        } else if let Some(mood) = inner.strip_prefix("mood:") {
            out.push(ServerMessage::Response(AgentResponse {
                text: String::new(),
                emotion: mood.trim().to_string(),
                audio_follows: true,
            }));
            buf.replace_range(open..close + 1, "");
            search = open;
        } else {
            search = close + 1; // leave unrecognized tag, skip past it
        }
    }
    out
}

fn parse_do(s: &str) -> Option<ServerMessage> {
    let (key, val) = s.split_once('=')?;
    let (key, val) = (key.trim(), val.trim());
    let msg = match key {
        "volume" => ControlMessage { action: "volume".into(), value: val.parse().ok(), color: None, dir: None },
        "led" => ControlMessage { action: "led".into(), value: None, color: Some(val.to_string()), dir: None },
        "turn" => ControlMessage { action: "turn".into(), value: None, color: None, dir: Some(val.to_string()) },
        _ => return None,
    };
    Some(ServerMessage::Control(msg))
}

/// Find the byte index just after the first usable sentence break, for incremental TTS.
fn split_sentence(s: &str) -> Option<usize> {
    const STRONG: [char; 6] = ['。', '！', '？', '!', '?', '\n'];
    const SOFT: [char; 4] = ['，', ',', '；', ';'];
    for (i, ch) in s.char_indices() {
        if STRONG.contains(&ch) {
            return Some(i + ch.len_utf8());
        }
    }
    // soft break only when enough has accumulated (keeps segments natural)
    for (i, ch) in s.char_indices() {
        if SOFT.contains(&ch) && s[..i].chars().count() >= 6 {
            return Some(i + ch.len_utf8());
        }
    }
    None
}

/// Speak a single canned/whole phrase as a complete reply turn.
async fn speak_phrase(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    session: &mut Session,
    text: &str,
    emotion: &str,
    audio: &[u8],
) {
    let response = AgentResponse {
        text: text.into(),
        emotion: emotion.into(),
        audio_follows: true,
    };
    for ev in session.transition_to_speaking(response) {
        send_event(sender, ev).await;
    }
    send_audio_stream(sender, audio, true).await;
    send_event(sender, SessionEvent::SendJson(ServerMessage::SpeakDone)).await;
    for ev in session.finish_speaking() {
        send_event(sender, ev).await;
    }
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
