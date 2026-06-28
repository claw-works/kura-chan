use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use crate::tasks::{now_unix, ScheduledTask, TaskAction};
use crate::ws::codec::{AudioFrame, AUDIO_OUTPUT, FLAG_START};
use crate::ws::protocol::{AgentResponse, ServerMessage, StateChange};
use crate::ws::session::SessionEvent;
use crate::ws::AppState;

const AUDIO_CHUNK: usize = 4096;
const DEFAULT_HEARTBEAT_SECS: u64 = 600;
const DEFAULT_JOB_POLL_SECS: u64 = 20;

/// Heartbeat interval (seconds). Env override `HEARTBEAT_SECS`, default 600.
/// This is an agent loop — slow on purpose; one tick may process many tasks.
fn heartbeat_secs() -> u64 {
    std::env::var("HEARTBEAT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n >= 10)
        .unwrap_or(DEFAULT_HEARTBEAT_SECS)
}

/// Job-scheduler poll interval (seconds). Env override `JOB_POLL_SECS`, default 20.
fn job_poll_secs() -> u64 {
    std::env::var("JOB_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n >= 5)
        .unwrap_or(DEFAULT_JOB_POLL_SECS)
}

/// Slow system-level heartbeat: online count, future health/credential hooks.
pub async fn run(state: Arc<AppState>) {
    let secs = heartbeat_secs();
    tracing::info!(interval_secs = secs, "heartbeat loop started");
    let mut tick = tokio::time::interval(Duration::from_secs(secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        heartbeat(&state).await;
    }
}

/// Business-level job scheduler: a fast poll loop. Each tick pulls due jobs
/// (atomically rescheduled in `take_due_jobs`) and spawns a task per job.
/// A per-device lock serializes a device's jobs while different devices run
/// concurrently (a device can only play one audio stream at a time).
pub async fn run_jobs(state: Arc<AppState>) {
    let secs = job_poll_secs();
    tracing::info!(interval_secs = secs, "job scheduler started");
    let mut tick = tokio::time::interval(Duration::from_secs(secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let due = crate::db::take_due_jobs(&state.db, now_unix()).await;
        if due.is_empty() {
            continue;
        }
        tracing::info!(count = due.len(), "dispatching due jobs");
        for job in due {
            if !state.registry.is_online(&job.device_id) {
                tracing::info!(job = %job.id, device = %job.device_id, "device offline; skip");
                continue;
            }
            let st = state.clone();
            tokio::spawn(async move {
                let lock = st.device_lock(&job.device_id).await;
                let _guard = lock.lock().await; // serialize this device's jobs
                if let Err(e) = execute(&st, &job).await {
                    tracing::error!(job = %job.id, error = %e, "job execution failed");
                }
            });
        }
    }
}

/// Self-maintenance pass. Lightweight; heavy work belongs in jobs.
async fn heartbeat(state: &Arc<AppState>) {
    let online = state.registry.online_count();
    tracing::info!(online_devices = online, "heartbeat");
    // Future hooks: credential refresh, downstream health probe, metrics, etc.
}

async fn execute(state: &Arc<AppState>, task: &ScheduledTask) -> Result<(), String> {
    let text = match &task.action {
        TaskAction::Say { text } => text.clone(),
        TaskAction::AgentPrompt { prompt } => run_agent(state, &task.device_id, prompt).await?,
        TaskAction::Workflow { name, params } => {
            let wf = state
                .workflow_store
                .get(name)
                .ok_or_else(|| format!("unknown workflow: {name}"))?;
            let prompt = wf.render(params);
            run_agent(state, &task.device_id, &prompt).await?
        }
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    speak_to_device(state, &task.device_id, text).await;
    Ok(())
}

/// Run a prompt through the harness as the device's actor (so memory + session
/// are unified with normal conversation), collect the full reply, log it.
async fn run_agent(state: &Arc<AppState>, device_id: &str, prompt: &str) -> Result<String, String> {
    let actor = crate::db::actor_by_device(&state.db, device_id)
        .await
        .ok_or_else(|| format!("no actor for device {device_id}"))?;
    let ttl = state.config.session.idle_new_session_secs as i64;
    let session = crate::db::get_or_create_session(&state.db, &actor.actor_id, ttl)
        .await
        .map_err(|e| e.to_string())?;
    let rules = state.config.agent.system_prompt.clone();
    let persona_full = format!("你的名字是{}。{}", actor.name, actor.persona.trim());
    let sp = format!("{}\n\n{}", persona_full.trim(), rules);
    let req = crate::llm::LlmRequest {
        system_prompt: sp,
        messages: vec![crate::llm::ChatMessage::user(prompt)],
        session_id: session.clone(),
        actor_id: actor.actor_id.clone(),
    };
    let mut stream = state.llm.stream(req).await.map_err(|e| format!("{e:?}"))?;
    let mut buf = String::new();
    loop {
        match stream.next().await {
            Some(Ok(t)) => buf.push_str(&t),
            Some(Err(e)) => return Err(e.to_string()),
            None => break,
        }
    }
    let clean = strip_tags(&buf);
    if !clean.is_empty() {
        crate::db::log_message(&state.db, &session, &actor.actor_id, "assistant", &clean).await;
        crate::db::touch_session(&state.db, &session).await;
    }
    Ok(clean)
}

/// Remove any [..] markers (mood/do/task) so proactive speech isn't read aloud.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        if c == '[' { depth += 1; }
        else if c == ']' { if depth > 0 { depth -= 1; } }
        else if depth == 0 { out.push(c); }
    }
    out.trim().to_string()
}

/// Proactively speak text to a connected device: TTS + push as a full turn
/// (state -> response -> audio frames -> speak_done -> idle).
async fn speak_to_device(state: &Arc<AppState>, device_id: &str, text: &str) {
    let voice = crate::db::actor_by_device(&state.db, device_id)
        .await
        .map(|a| a.voice)
        .unwrap_or_default();
    let audio = state.tts.synthesize(text, crate::speech::voice_id(&voice)).await.unwrap_or_default();
    if audio.is_empty() {
        tracing::warn!(device = device_id, "TTS produced no audio; skip push");
        return;
    }
    let reg = &state.registry;
    reg.push(device_id, SessionEvent::SendJson(ServerMessage::State(StateChange {
        state: "speaking".into(),
    })));
    reg.push(device_id, SessionEvent::SendJson(ServerMessage::Response(AgentResponse {
        text: text.to_string(),
        emotion: "neutral".into(),
        audio_follows: true,
    })));
    let mut off = 0;
    let mut first = true;
    while off < audio.len() {
        let end = (off + AUDIO_CHUNK).min(audio.len());
        let flags = if first { FLAG_START } else { 0 };
        first = false;
        let frame = AudioFrame {
            frame_type: AUDIO_OUTPUT,
            flags,
            payload: audio[off..end].to_vec(),
        };
        reg.push(device_id, SessionEvent::SendAudio(frame.encode()));
        off = end;
    }
    reg.push(device_id, SessionEvent::SendJson(ServerMessage::SpeakDone));
    reg.push(device_id, SessionEvent::SendJson(ServerMessage::State(StateChange {
        state: "idle".into(),
    })));
    tracing::info!(device = device_id, bytes = audio.len(), "pushed proactive speech");
}
