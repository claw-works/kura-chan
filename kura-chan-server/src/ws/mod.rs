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
use crate::llm::{ChatMessage, LlmRequest, Role};
use crate::speech::{SpeechToText, TextToSpeech};
use crate::ws::codec::{AudioFrame, AUDIO_OUTPUT, FLAG_START};
use crate::ws::protocol::*;
use crate::ws::session::{Session, SessionEvent, SessionState};

pub struct AppState {
    pub config: Arc<Config>,
    pub llm: Box<dyn crate::llm::LlmProvider>,
    pub stt: Box<dyn SpeechToText>,
    pub tts: Box<dyn TextToSpeech>,
    /// Cache of synthesized canned phrases (phrase text -> PCM), so common
    /// replies like "没听清" are only synthesized once.
    pub canned: tokio::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    /// Connected devices, for proactive push from background tasks.
    pub registry: Arc<crate::registry::SessionRegistry>,
    /// Per-device async locks so a device's scheduled jobs run serially
    /// (one audio stream at a time), while different devices run concurrently.
    pub device_locks: tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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
    /// Get (or create) the per-device serialization lock.
    pub async fn device_lock(&self, device_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.device_locks.lock().await;
        map.entry(device_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Get cached PCM for a canned phrase, synthesizing+caching on first use.
    pub async fn canned_audio(&self, text: &str, voice: &str) -> Vec<u8> {
        let key = format!("{voice}\u{1}{text}");
        if let Some(a) = self.canned.lock().await.get(&key) {
            return a.clone();
        }
        let audio = self.tts.synthesize(text, voice).await.unwrap_or_default();
        if !audio.is_empty() {
            self.canned.lock().await.insert(key, audio.clone());
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

    // system prompt is rebuilt per turn (see invoke below), so admin/PG edits and
    // bond/level changes apply immediately without a device reconnect.
    let session_ttl = state.config.session.idle_new_session_secs as i64;
    let growth = state.config.growth.clone();

    // settle passive energy regen (time since last seen), then push full state on connect
    if let Some(a) =
        db::bump_growth(&state.db, &actor.actor_id, 0, 0, 0, growth.xp_base, growth.energy_regen_per_hour).await
    {
        actor = a;
    }
    send_event(&tx, sync_msg(&actor, true, growth.xp_base)).await;

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
                match serde_json::from_str::<ClientMessage>(&text) {
                    // Text input (desktop, no mic): same turn pipeline, skip STT.
                    Ok(ClientMessage::TextInput(ti)) => {
                        let t = ti.text.trim().to_string();
                        if !t.is_empty() {
                            run_turn(&state, &tx, &mut session, &mut actor, &device_id, &growth, session_ttl, &t).await;
                        }
                    }
                    Ok(ClientMessage::Hello(hello)) => {
                        for ev in session.handle_hello(hello) {
                            send_event(&tx, ev).await;
                        }
                    }
                    Ok(ClientMessage::ToolResult(result)) => {
                        for ev in session.handle_tool_result(result) {
                            send_event(&tx, ev).await;
                        }
                    }
                    Ok(ClientMessage::Status(status)) => {
                        if let Some(ap) = &status.appearance {
                            let mut ap2 = ap.clone();
                            if let Some(o) = ap2.as_object_mut() { o.remove("bg"); }  // bg is server-owned
                            db::set_appearance(&state.db, &actor.actor_id, &ap2).await;
                        }
                        for ev in session.handle_status(status) {
                            send_event(&tx, ev).await;
                        }
                    }
                    Ok(ClientMessage::Event(ev)) => {
                        // head_pat is the chat wake trigger only — no growth (too easy to farm)
                        let _ = ev;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Invalid JSON message");
                        send_event(&tx, SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                            code: "parse_error".into(),
                            message: e.to_string(),
                        }))).await;
                    }
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
                    // Empty STT (silence/noise) → canned phrase; still device-locked
                    // so it can't clash with a scheduled job's audio.
                    if stt_text.trim().is_empty() {
                        tracing::info!("Empty STT, playing canned phrase");
                        let dev_lock = state.device_lock(&device_id).await;
                        let _g = dev_lock.lock().await;
                        let audio = state.canned_audio(PHRASE_NOT_HEARD, crate::speech::voice_id(&actor.voice)).await;
                        speak_phrase(&tx, &mut session, PHRASE_NOT_HEARD, "confused", &audio).await;
                        continue;
                    }
                    run_turn(&state, &tx, &mut session, &mut actor, &device_id, &growth, session_ttl, &stt_text).await;
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

/// Run one conversation turn: device-locked LLM stream → sentence-by-sentence
/// TTS (audio + subtitle) → tag handling → growth settle → SpeakDone. Shared by
/// voice (post-STT) and text input. `user_text` is the user's utterance/message
/// (already non-empty).
async fn run_turn(
    state: &Arc<AppState>,
    tx: &crate::registry::DeviceTx,
    session: &mut Session,
    actor: &mut Actor,
    device_id: &str,
    growth: &crate::config::GrowthConfig,
    session_ttl: i64,
    user_text: &str,
) {
    // Hold this device's lock for the whole reply turn so a scheduled job
    // (run_jobs) can't push audio into the device mid-conversation.
    let dev_lock = state.device_lock(device_id).await;
    let _turn_guard = dev_lock.lock().await;

    // Echo the user input back (transcript / typed text) for clients that show it.
    send_event(
        tx,
        SessionEvent::SendJson(ServerMessage::Stt(SttResult {
            text: user_text.to_string(),
            r#final: true,
        })),
    )
    .await;

    let conv_session = db::get_or_create_session(&state.db, &actor.actor_id, session_ttl)
        .await
        .unwrap_or_else(|_| session.session_id.clone());
    let history = db::get_recent_messages(
        &state.db,
        &conv_session,
        &actor.actor_id,
        (state.config.llm.history_turns as i64) * 2,
    )
    .await;
    db::log_message(&state.db, &conv_session, &actor.actor_id, "user", user_text).await;

    let user_message = session.build_user_message(user_text);
    // Rebuild system prompt each turn from the latest actor + PG content so admin
    // edits and bond/level changes apply immediately.
    if let Some(a) = db::actor_by_id(&state.db, &actor.actor_id).await {
        *actor = a;
    }
    let system_prompt = build_system_prompt(state, actor).await;
    let mut messages: Vec<ChatMessage> = history
        .into_iter()
        .map(|m| ChatMessage {
            role: if m.role == "assistant" { Role::Assistant } else { Role::User },
            content: m.content,
        })
        .collect();
    messages.push(ChatMessage::user(user_message));
    let req = LlmRequest {
        system_prompt,
        messages,
        session_id: conv_session.clone(),
        actor_id: actor.actor_id.clone(),
    };
    let mut stream = match state.llm.stream(req).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "LLM invoke failed");
            let audio = state.canned_audio(PHRASE_ERROR, crate::speech::voice_id(&actor.voice)).await;
            speak_phrase(tx, session, PHRASE_ERROR, "sad", &audio).await;
            return;
        }
    };

    for ev in session.transition_to_speaking(AgentResponse {
        text: String::new(),
        emotion: "happy".into(),
        audio_follows: true,
    }) {
        send_event(tx, ev).await;
    }

    let mut buf = String::new();
    let mut new_tasks: Vec<crate::tasks::ScheduledTask> = Vec::new();
    let mut cancel_jobs: Vec<String> = Vec::new();
    let mut appearance_ops: Vec<(String, serde_json::Value)> = Vec::new();
    let mut turn_growth = TurnGrowth::default();
    let mut reply_text = String::new();
    let mut raw_reply = String::new();
    let mut want_new_session = false;
    let mut first = true;
    let mut sent_any = false;
    loop {
        match stream.next().await {
            Some(Ok(t)) => {
                raw_reply.push_str(&t);
                buf.push_str(&t);
                for msg in extract_tags(&mut buf, device_id, &mut new_tasks, &mut want_new_session, &mut appearance_ops, &mut turn_growth, &mut cancel_jobs) {
                    send_event(tx, SessionEvent::SendJson(msg)).await;
                }
                while let Some(cut) = {
                    let safe = buf.find('[').unwrap_or(buf.len());
                    split_sentence(&buf[..safe])
                } {
                    let seg: String = buf.drain(..cut).collect();
                    let seg = seg.trim();
                    if !seg.is_empty() && !seg.contains("[NOISE]") {
                        reply_text.push_str(seg);
                        send_event(tx, SessionEvent::SendJson(ServerMessage::Subtitle(Subtitle {
                            text: seg.to_string(),
                            r#final: false,
                        }))).await;
                        let audio = state.tts.synthesize(seg, crate::speech::voice_id(&actor.voice)).await.unwrap_or_default();
                        if !audio.is_empty() {
                            send_audio_stream(tx, &audio, first).await;
                            first = false;
                            sent_any = true;
                        }
                    }
                }
            }
            Some(Err(e)) => {
                tracing::error!(error = %e, "LLM stream error");
                break;
            }
            None => break,
        }
    }
    for msg in extract_tags(&mut buf, device_id, &mut new_tasks, &mut want_new_session, &mut appearance_ops, &mut turn_growth, &mut cancel_jobs) {
        send_event(tx, SessionEvent::SendJson(msg)).await;
    }
    let rest = buf.trim().to_string();
    if !rest.is_empty() && !rest.contains("[NOISE]") {
        reply_text.push_str(&rest);
        send_event(tx, SessionEvent::SendJson(ServerMessage::Subtitle(Subtitle {
            text: rest.clone(),
            r#final: false,
        }))).await;
        let audio = state.tts.synthesize(&rest, crate::speech::voice_id(&actor.voice)).await.unwrap_or_default();
        if !audio.is_empty() {
            send_audio_stream(tx, &audio, first).await;
            first = false;
            sent_any = true;
        }
    }
    // subtitle end-of-turn marker (clients finalize the bubble)
    send_event(tx, SessionEvent::SendJson(ServerMessage::Subtitle(Subtitle {
        text: String::new(),
        r#final: true,
    }))).await;
    tracing::info!(raw = %raw_reply, "🧠 LLM raw output");
    tracing::info!(spoken = %reply_text, "🔊 TTS text");
    if !sent_any {
        tracing::info!("No speakable reply, using canned phrase");
        let audio = state.canned_audio(PHRASE_NOT_HEARD, crate::speech::voice_id(&actor.voice)).await;
        send_audio_stream(tx, &audio, first).await;
    }
    for t in new_tasks {
        match db::add_job(&state.db, &actor.actor_id, &t).await {
            Ok(id) => tracing::info!(job = %id, device = %device_id, "job created via voice"),
            Err(e) => tracing::error!(error = %e, "job create failed"),
        }
    }
    for id in cancel_jobs {
        if db::delete_job(&state.db, &id).await {
            tracing::info!(job = %id, "job cancelled via voice");
        }
    }
    let assistant_msg = raw_reply.trim().to_string();
    if !assistant_msg.is_empty() {
        db::log_message(&state.db, &conv_session, &actor.actor_id, "assistant", &assistant_msg).await;
    }
    db::touch_session(&state.db, &conv_session).await;
    for (k, v) in &appearance_ops {
        db::set_appearance_key(&state.db, &actor.actor_id, k, v.clone()).await;
    }
    let event_xp: i32 = turn_growth.events.iter().map(|lvl| growth.event_xp(lvl)).sum();
    let dxp = growth.base_xp + event_xp;
    let dbond = turn_growth.bond.clamp(-(growth.bond_max_delta * 3), growth.bond_max_delta);
    if let Some(a) = db::bump_growth(
        &state.db,
        &actor.actor_id,
        dxp,
        dbond,
        -growth.turn_energy,
        growth.xp_base,
        growth.energy_regen_per_hour,
    )
    .await
    {
        *actor = a;
        send_event(tx, sync_msg(actor, false, growth.xp_base)).await;
    }
    if want_new_session {
        if let Ok(sid) = db::new_session(&state.db, &actor.actor_id).await {
            tracing::info!(actor = %actor.actor_id, session = %sid, "new session (agent requested)");
        }
    }
    send_event(tx, SessionEvent::SendJson(ServerMessage::SpeakDone)).await;
    for ev in session.finish_speaking() {
        send_event(tx, ev).await;
    }
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
/// Growth signals the agent emits during a turn (stripped from spoken text).
#[derive(Default)]
struct TurnGrowth {
    /// event level names ([event:minor|major|epic]) -> resolved to XP later
    events: Vec<String>,
    /// accumulated intimacy delta from [bond:+N]/[bond:-N]
    bond: i32,
}

fn extract_tags(
    buf: &mut String,
    device_id: &str,
    tasks: &mut Vec<crate::tasks::ScheduledTask>,
    new_session: &mut bool,
    apps: &mut Vec<(String, serde_json::Value)>,
    growth: &mut TurnGrowth,
    cancels: &mut Vec<String>,
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
            // server-owned appearance keys (persisted to the actor)
            if let Some((k, v)) = rest.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                let b = v == "on" || v == "1" || v == "true";
                match k {
                    "bg" => apps.push(("bg".into(), serde_json::Value::String(v.to_string()))),
                    "blush" => apps.push(("blush".into(), serde_json::json!(b))),
                    "glasses" => apps.push(("glasses".into(), serde_json::json!(b))),
                    _ => {}
                }
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
            if let Some(id) = rest.trim().strip_prefix("cancel=") {
                // tolerate a leading '#' (the injected list shows ids as "#id")
                cancels.push(id.trim().trim_start_matches('#').trim().to_string());
            } else if let Some(t) = parse_task(rest, device_id) {
                tasks.push(t);
            }
            buf.replace_range(open..close + 1, "");
            search = open;
        } else if let Some(rest) = inner.strip_prefix("event:") {
            growth.events.push(rest.trim().to_string());
            buf.replace_range(open..close + 1, "");
            search = open;
        } else if let Some(rest) = inner.strip_prefix("bond:") {
            if let Ok(n) = rest.trim().trim_start_matches('+').parse::<i32>() {
                growth.bond += n;
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
                    "in" | "cron" => {} // schedule keys
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

/// Parse the schedule: `cron=<5 fields>` (recurring) or `in=N` (one-shot, N secs from now).
fn parse_schedule(s: &str) -> Option<crate::tasks::Schedule> {
    use crate::tasks::{cron_valid, now_unix, Schedule};
    if let Some(i) = s.find("cron=") {
        // a standard cron expression is 5 whitespace-separated fields
        let fields: Vec<&str> = s[i + 5..].split_whitespace().take(5).collect();
        if fields.len() == 5 {
            let expr = fields.join(" ");
            if cron_valid(&expr) {
                return Some(Schedule::Cron { expr });
            }
        }
        return None;
    }
    if let Some(secs) = grab_num(s, "in=") {
        return Some(Schedule::Once { at: now_unix() + secs as i64 });
    }
    None
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
        "bg" => ControlMessage { action: "bg".into(), value: None, color: None, dir: None, name: Some(val.to_string()) },
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

/// Assemble the per-actor system prompt from PG (all level/bond gated):
/// persona base + common rules + unlocked spirit fragments (rule=always,
/// persona/topic/boundary=highest unlocked tier) + relationship state + options.
async fn build_system_prompt(state: &AppState, actor: &Actor) -> String {
    let persona = format!("你的名字是{}。{}", actor.name, actor.persona.trim());
    let rules = db::get_prompt_template(&state.db, "common_rules")
        .await
        .unwrap_or_else(|| state.config.agent.system_prompt.clone());
    let frags = db::get_fragments(&state.db, &actor.actor_id, actor.level, actor.bond).await;
    let mut best: std::collections::HashMap<&str, &db::PromptFragment> =
        std::collections::HashMap::new();
    let mut rule_lines: Vec<&str> = Vec::new();
    for f in &frags {
        if f.kind == "rule" {
            rule_lines.push(f.content.as_str());
            continue;
        }
        match best.get(f.kind.as_str()) {
            Some(c) if c.min_bond >= f.min_bond => {}
            _ => {
                best.insert(f.kind.as_str(), f);
            }
        }
    }
    let mut spirit: Vec<String> = rule_lines.iter().map(|s| s.to_string()).collect();
    for k in ["persona", "topic", "boundary"] {
        if let Some(f) = best.get(k) {
            spirit.push(f.content.clone());
        }
    }
    let rel = format!("【当前状态】等级 Lv{}，亲密度 {}/100。", actor.level, actor.bond);
    let options =
        crate::assets::options_prompt(&state.db, &actor.gender, actor.level, actor.bond).await;
    // Active jobs so the agent can answer "what reminders do I have" and cancel them.
    let jobs = db::list_jobs(&state.db, &actor.actor_id).await;
    let jobs_txt = if jobs.is_empty() {
        "【当前定时任务】（无）".to_string()
    } else {
        let lines: Vec<String> = jobs
            .iter()
            .map(|j| format!("#{} {} — {}", j.id, describe_schedule(&j.schedule), describe_action(&j.action)))
            .collect();
        format!(
            "【当前定时任务】(用户要取消某个时，输出 [task:cancel=对应ID])\n{}",
            lines.join("\n")
        )
    };
    // Critical output-format reminder, placed LAST so the model reliably follows it
    // even with a long prompt (instructions buried mid-prompt get ignored).
    let format_reminder = "————\n【输出格式·务必遵守】\n\
        1. 每条回复都以情绪标记开头，如 [mood:happy]。\n\
        2. 需要换装/发型/脸红/眼镜时输出 [do:wear=变体] / [do:blush=on] / [do:glasses=on]；\
        去某地玩或换场景时输出 [do:bg=场景名]（场景名只能取上面【当前可用项】里列出的）。\n\
        3. 按本轮互动用 [bond:+N] / [bond:-N]（节制）；遇到重要时刻用 [event:minor|major|epic]。\n\
        这些方括号标记是给系统执行的、不会被朗读；不输出标记＝对应功能不会发生。";
    format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}",
        persona.trim(),
        rules.trim(),
        spirit.join("\n"),
        rel,
        options,
        jobs_txt,
        format_reminder
    )
}

fn describe_schedule(s: &crate::tasks::Schedule) -> String {
    use crate::tasks::Schedule::*;
    match s {
        Once { at } => {
            use chrono::{FixedOffset, TimeZone};
            FixedOffset::east_opt(8 * 3600)
                .and_then(|bj| bj.timestamp_opt(*at, 0).single())
                .map(|dt| format!("一次性 {}", dt.format("%m-%d %H:%M")))
                .unwrap_or_else(|| "一次性".to_string())
        }
        Cron { expr } => format!("cron[{expr}]"),
    }
}

fn describe_action(a: &crate::tasks::TaskAction) -> String {
    use crate::tasks::TaskAction::*;
    let s = match a {
        Say { text } => text.clone(),
        AgentPrompt { prompt } => prompt.clone(),
        Workflow { name, .. } => format!("workflow:{name}"),
    };
    s.chars().take(24).collect()
}

/// Build a Sync event. `full` includes appearance (used on connect to restore);
/// growth-only syncs omit it so they don't revert a just-applied outfit change.
fn sync_msg(a: &Actor, full: bool, xp_base: i32) -> SessionEvent {
    SessionEvent::SendJson(ServerMessage::Sync(SyncState {
        gender: a.gender.clone(),
        appearance: if full { Some(a.appearance.clone()) } else { None },
        level: a.level,
        xp: a.xp,
        xp_need: db::xp_need(a.level, xp_base),
        bond: a.bond,
        energy: a.energy,
    }))
}
