pub mod codec;
pub mod protocol;
pub mod session;

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};

use crate::config::Config;
use crate::db::{self, Actor};
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
    /// Connected devices, for proactive push from background tasks.
    pub registry: Arc<crate::registry::SessionRegistry>,
    /// User-defined scheduled tasks (file-backed).
    pub task_store: Arc<crate::tasks::TaskStore>,
    /// Reusable request templates run through the harness (file-backed).
    pub workflow_store: Arc<crate::workflows::WorkflowStore>,
    /// Postgres pool (actors / sessions / messages).
    pub db: crate::db::Db,
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
    let actor = match crate::auth::authenticate(&headers, &state.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "Auth failed");
            return Err(AppError::Auth(e));
        }
    };
    // registry key: the device id sent by the client (tasks reference this), else actor's.
    let device_id = headers
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| actor.device_id.clone())
        .unwrap_or_else(|| actor.actor_id.clone());

    tracing::info!(device_id = %device_id, actor = %actor.actor_id, "WebSocket upgrade accepted");
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, device_id, actor, state)))
}

async fn handle_socket(socket: WebSocket, device_id: String, mut actor: Actor, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    // Outbound channel + writer task. The receive loop and background tasks
    // (heartbeat/scheduler, via the registry) both push events through `tx`;
    // the writer owns the sink and serializes them onto the socket.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
    state.registry.register(&device_id, tx.clone());
    let writer = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let msg = match ev {
                SessionEvent::SendJson(m) => match serde_json::to_string(&m) {
                    Ok(j) => Message::Text(j.into()),
                    Err(_) => continue,
                },
                SessionEvent::SendAudio(d) => Message::Binary(d.into()),
            };
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut session = Session::new(device_id.clone(), state.config.clone());

    // per-actor system prompt = name + persona prefix + common rules (from config)
    let rules = state.config.agent.system_prompt.clone();
    let persona_full = format!("你的名字是{}。{}", actor.name, actor.persona.trim());
    let system_prompt = format!("{}\n\n{}", persona_full.trim(), rules);
    let session_ttl = state.config.session.idle_new_session_secs as i64;

    // push server-authoritative state to the device on connect
    send_event(&tx, sync_msg(&actor)).await;

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
                    Ok(ClientMessage::Status(status)) => {
                        if let Some(ap) = &status.appearance {
                            db::set_appearance(&state.db, &actor.actor_id, ap).await;
                        }
                        session.handle_status(status)
                    }
                    Ok(ClientMessage::Event(ev)) => {
                        if ev.kind == "head_pat" {
                            if let Some(a) = db::bump_growth(&state.db, &actor.actor_id, 3, 3, 0).await {
                                actor = a;
                                send_event(&tx, sync_msg(&actor)).await;
                            }
                        }
                        vec![]
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Invalid JSON message");
                        vec![SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                            code: "parse_error".into(),
                            message: e.to_string(),
                        }))]
                    }
                };
                for event in events {
                    send_event(&tx, event).await;
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
                    send_event(&tx, event).await;
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
                        &tx,
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
                        speak_phrase(&tx, &mut session, PHRASE_NOT_HEARD, "confused", &audio).await;
                        continue;
                    }

                    // Resolve the actor's conversation session (rolls over after idle TTL).
                    let conv_session = db::get_or_create_session(&state.db, &actor.actor_id, session_ttl)
                        .await
                        .unwrap_or_else(|_| session.session_id.clone());
                    db::log_message(&state.db, &conv_session, &actor.actor_id, "user", &stt_text).await;

                    // Stream the harness reply: synthesize + send sentence by sentence.
                    let user_message = session.build_user_message(&stt_text);
                    let mut output = match state
                        .harness
                        .invoke_stream(&user_message, &conv_session, &actor.actor_id, &system_prompt)
                        .await
                    {
                        Ok(o) => o,
                        Err(e) => {
                            tracing::error!(error = ?e, "Harness invoke failed");
                            let audio = state.canned_audio(PHRASE_ERROR).await;
                            speak_phrase(&tx, &mut session, PHRASE_ERROR, "sad", &audio).await;
                            continue;
                        }
                    };

                    for ev in session.transition_to_speaking(AgentResponse {
                        text: String::new(),
                        emotion: "happy".into(),
                        audio_follows: true,
                    }) {
                        send_event(&tx, ev).await;
                    }

                    let mut buf = String::new();
                    let mut new_tasks: Vec<crate::tasks::ScheduledTask> = Vec::new();
                    let mut reply_text = String::new();
                    let mut want_new_session = false;
                    let mut first = true;
                    let mut sent_any = false;
                    loop {
                        match output.stream.recv().await {
                            Ok(Some(event)) => {
                                if let Some(t) = extract_text_delta(&event) {
                                    buf.push_str(&t);
                                    for msg in extract_tags(&mut buf, &device_id, &mut new_tasks, &mut want_new_session) {
                                        send_event(&tx, SessionEvent::SendJson(msg)).await;
                                    }
                                    while let Some(cut) = split_sentence(&buf) {
                                        let seg: String = buf.drain(..cut).collect();
                                        let seg = seg.trim();
                                        if !seg.is_empty() && !seg.contains("[NOISE]") {
                                            reply_text.push_str(seg);
                                            let audio =
                                                state.tts.synthesize(seg).await.unwrap_or_default();
                                            if !audio.is_empty() {
                                                send_audio_stream(&tx, &audio, first).await;
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
                    for msg in extract_tags(&mut buf, &device_id, &mut new_tasks, &mut want_new_session) {
                        send_event(&tx, SessionEvent::SendJson(msg)).await;
                    }
                    let rest = buf.trim().to_string();
                    if !rest.is_empty() && !rest.contains("[NOISE]") {
                        reply_text.push_str(&rest);
                        let audio = state.tts.synthesize(&rest).await.unwrap_or_default();
                        if !audio.is_empty() {
                            send_audio_stream(&tx, &audio, first).await;
                            first = false;
                            sent_any = true;
                        }
                    }
                    // nothing intelligible → canned "not heard"
                    if !sent_any {
                        tracing::info!("No speakable reply, using canned phrase");
                        let audio = state.canned_audio(PHRASE_NOT_HEARD).await;
                        send_audio_stream(&tx, &audio, first).await;
                    }
                    // Persist any tasks the agent decided to create this turn.
                    for t in new_tasks {
                        let id = t.id.clone();
                        state.task_store.add(t);
                        tracing::info!(task = %id, device = %device_id, "task created via voice");
                    }
                    // log assistant reply + roll session activity / reset
                    let reply_text = reply_text.trim().to_string();
                    if !reply_text.is_empty() {
                        db::log_message(&state.db, &conv_session, &actor.actor_id, "assistant", &reply_text).await;
                    }
                    db::touch_session(&state.db, &conv_session).await;
                    // server-authoritative growth: a completed turn earns xp/bond, costs energy
                    if let Some(a) = db::bump_growth(&state.db, &actor.actor_id, 12, 2, -4).await {
                        actor = a;
                        send_event(&tx, sync_msg(&actor)).await;
                    }
                    if want_new_session {
                        if let Ok(sid) = db::new_session(&state.db, &actor.actor_id).await {
                            tracing::info!(actor = %actor.actor_id, session = %sid, "new session (agent requested)");
                        }
                    }
                    send_event(&tx, SessionEvent::SendJson(ServerMessage::SpeakDone))
                        .await;
                    for ev in session.finish_speaking() {
                        send_event(&tx, ev).await;
                    }
                }
            }
            Message::Ping(_) => {}
            Message::Close(_) => break,
            _ => {}
        }
    }

    state.registry.unregister(&device_id);
    drop(tx);
    let _ = writer.await;
    tracing::info!(device_id = %device_id, "Device disconnected");
}

/// Push PCM as AUDIO_OUTPUT frames (chunked). `start` marks the very first frame
/// of a reply (device resets its playback buffer and switches to the speaker).
/// No END flag is used; the reply ends with a SpeakDone control message.
async fn send_audio_stream(
    tx: &crate::registry::DeviceTx,
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
        send_event(tx, SessionEvent::SendAudio(frame.encode())).await;
        off = end;
    }
}

/// Remove complete `[do:...]` / `[mood:...]` tags from `buf`, returning the
/// control/emotion messages to send. Unrecognized tags (e.g. `[NOISE]`) and any
/// unclosed trailing `[...` are left in place so TTS handling stays correct.
fn extract_tags(
    buf: &mut String,
    device_id: &str,
    tasks: &mut Vec<crate::tasks::ScheduledTask>,
    new_session: &mut bool,
) -> Vec<ServerMessage> {
    let mut out = Vec::new();
    let mut search = 0;
    loop {
        let Some(rel) = buf[search..].find('[') else { break };
        let open = search + rel;
        let Some(crel) = buf[open..].find(']') else { break }; // unclosed: wait for more
        let close = open + crel;
        let inner = buf[open + 1..close].trim().to_string();
        if let Some(rest) = inner.strip_prefix("do:") {
            if rest.trim() == "newsession" {
                *new_session = true;
            } else if let Some(msg) = parse_do(rest) {
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
        } else if let Some(rest) = inner.strip_prefix("task:") {
            if let Some(t) = parse_task(rest, device_id) {
                tasks.push(t);
            }
            buf.replace_range(open..close + 1, "");
            search = open;
        } else {
            search = close + 1; // leave unrecognized tag, skip past it
        }
    }
    out
}

/// Parse a `[task:...]` marker the agent emits to schedule a reminder/workflow:
///   [task:in=3600 say=该喝水啦]              one-shot reminder (relative secs)
///   [task:every=3600 say=起来动一动]          recurring
///   [task:daily=09:00 say=早安主人]           every day at 09:00 (Beijing)
///   [task:daily=09:00 workflow=weather_report city=北京]  run a workflow daily
/// `say=`/`ask=` text runs to the end (may contain spaces/CJK); workflow params
/// are single-token `key=value` pairs.
fn parse_task(s: &str, device_id: &str) -> Option<crate::tasks::ScheduledTask> {
    use crate::tasks::{ScheduledTask, TaskAction};
    let s = s.trim();
    let (action, sched_src): (TaskAction, &str) = if let Some(i) = s.find("say=") {
        (TaskAction::Say { text: s[i + 4..].trim().to_string() }, &s[..i])
    } else if let Some(i) = s.find("ask=") {
        (TaskAction::AgentPrompt { prompt: s[i + 4..].trim().to_string() }, &s[..i])
    } else if s.contains("workflow=") {
        let mut name: Option<String> = None;
        let mut params = std::collections::BTreeMap::new();
        for tok in s.split_whitespace() {
            if let Some((k, v)) = tok.split_once('=') {
                match k {
                    "workflow" => name = Some(v.to_string()),
                    "in" | "every" | "daily" => {} // schedule keys
                    _ => {
                        params.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        (TaskAction::Workflow { name: name?, params }, s)
    } else {
        return None;
    };
    let schedule = parse_schedule(sched_src)?;
    match &action {
        TaskAction::Say { text } if text.is_empty() => return None,
        TaskAction::AgentPrompt { prompt } if prompt.is_empty() => return None,
        _ => {}
    }
    Some(ScheduledTask::new(device_id.to_string(), action, schedule))
}

/// Parse the schedule from the tokens (`daily=HH:MM` / `every=N` / `in=N`).
fn parse_schedule(s: &str) -> Option<crate::tasks::Schedule> {
    use crate::tasks::{now_unix, Schedule};
    if let Some((h, m)) = grab_daily(s) {
        return Some(Schedule::Daily { hour: h, minute: m });
    }
    if let Some(secs) = grab_num(s, "every=") {
        return Some(Schedule::Interval { secs });
    }
    if let Some(secs) = grab_num(s, "in=") {
        return Some(Schedule::Once { at: now_unix() + secs as i64 });
    }
    None
}

/// Parse `daily=HH:MM` -> (hour, minute).
fn grab_daily(s: &str) -> Option<(u32, u32)> {
    let i = s.find("daily=")? + 6;
    let tok: String = s[i..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let (h, m) = tok.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

/// Grab the unsigned integer following `key` in `s` (e.g. key="in=" -> 3600).
fn grab_num(s: &str, key: &str) -> Option<u64> {
    let i = s.find(key)? + key.len();
    let digits: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn parse_do(s: &str) -> Option<ServerMessage> {
    let (key, val) = s.split_once('=')?;
    let (key, val) = (key.trim(), val.trim());
    let on = |v: &str| if v == "on" || v == "1" || v == "true" { 1 } else { 0 };
    let msg = match key {
        "volume" => ControlMessage { action: "volume".into(), value: val.parse().ok(), color: None, dir: None, name: None },
        "led" => ControlMessage { action: "led".into(), value: None, color: Some(val.to_string()), dir: None, name: None },
        "turn" => ControlMessage { action: "turn".into(), value: None, color: None, dir: Some(val.to_string()), name: None },
        "wear" => ControlMessage { action: "wear".into(), value: None, color: None, dir: None, name: Some(val.to_string()) },
        "blush" => ControlMessage { action: "blush".into(), value: Some(on(val)), color: None, dir: None, name: None },
        "glasses" => ControlMessage { action: "glasses".into(), value: Some(on(val)), color: None, dir: None, name: None },
        "char" => ControlMessage { action: "char".into(), value: None, color: None, dir: None, name: Some(val.to_string()) },
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
    tx: &crate::registry::DeviceTx,
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
        send_event(tx, ev).await;
    }
    send_audio_stream(tx, audio, true).await;
    send_event(tx, SessionEvent::SendJson(ServerMessage::SpeakDone)).await;
    for ev in session.finish_speaking() {
        send_event(tx, ev).await;
    }
}

async fn send_event(tx: &crate::registry::DeviceTx, event: SessionEvent) {
    // Hand the event to the per-device writer task (non-blocking).
    let _ = tx.send(event);
}

/// Build a Sync event from the actor's current server-authoritative state.
fn sync_msg(a: &Actor) -> SessionEvent {
    SessionEvent::SendJson(ServerMessage::Sync(SyncState {
        gender: a.gender.clone(),
        appearance: a.appearance.clone(),
        level: a.level,
        xp: a.xp,
        xp_need: db::xp_need(a.level),
        bond: a.bond,
        energy: a.energy,
    }))
}
