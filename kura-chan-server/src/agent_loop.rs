use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::harness::invoke::extract_text_delta;
use crate::tasks::{now_unix, ScheduledTask, TaskAction};
use crate::ws::codec::{AudioFrame, AUDIO_OUTPUT, FLAG_START};
use crate::ws::protocol::{AgentResponse, ServerMessage, StateChange};
use crate::ws::session::SessionEvent;
use crate::ws::AppState;

const AUDIO_CHUNK: usize = 4096;
const DEFAULT_HEARTBEAT_SECS: u64 = 600;

/// Heartbeat interval (seconds). Env override `HEARTBEAT_SECS`, default 600.
/// This is an agent loop — slow on purpose; one tick may process many tasks.
fn heartbeat_secs() -> u64 {
    std::env::var("HEARTBEAT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n >= 10)
        .unwrap_or(DEFAULT_HEARTBEAT_SECS)
}

/// The main agent loop: a slow heartbeat that self-maintains and drives the
/// scheduled-task system. Spawned once at startup.
pub async fn run(state: Arc<AppState>) {
    let secs = heartbeat_secs();
    tracing::info!(interval_secs = secs, "agent loop started");
    let mut tick = tokio::time::interval(Duration::from_secs(secs));
    // skip the immediate first tick burst
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        heartbeat(&state).await;
        process_due_tasks(&state).await;
    }
}

/// Self-maintenance pass. Lightweight; heavy work belongs in tasks.
async fn heartbeat(state: &Arc<AppState>) {
    let online = state.registry.online_count();
    let tasks = state.task_store.list().len();
    tracing::info!(online_devices = online, tasks, "heartbeat");
    // Future hooks: credential refresh, downstream health probe, metrics, etc.
}

/// Fire all tasks due now. Offline devices' tasks are skipped (already
/// rescheduled by take_due; interval tasks will retry next tick).
async fn process_due_tasks(state: &Arc<AppState>) {
    let due = state.task_store.take_due(now_unix());
    if due.is_empty() {
        return;
    }
    tracing::info!(count = due.len(), "processing due tasks");
    for task in due {
        if !state.registry.is_online(&task.device_id) {
            tracing::info!(task = %task.id, device = %task.device_id, "device offline; skip");
            continue;
        }
        if let Err(e) = execute(state, &task).await {
            tracing::error!(task = %task.id, error = %e, "task execution failed");
        }
    }
}

async fn execute(state: &Arc<AppState>, task: &ScheduledTask) -> Result<(), String> {
    let text = match &task.action {
        TaskAction::Say { text } => text.clone(),
        TaskAction::AgentPrompt { prompt } => run_agent(state, prompt).await?,
        TaskAction::Workflow { name, params } => {
            let wf = state
                .workflow_store
                .get(name)
                .ok_or_else(|| format!("unknown workflow: {name}"))?;
            let prompt = wf.render(params);
            run_agent(state, &prompt).await?
        }
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    speak_to_device(state, &task.device_id, text).await;
    Ok(())
}

/// Run a prompt through the harness and collect the full reply text.
async fn run_agent(state: &Arc<AppState>, prompt: &str) -> Result<String, String> {
    let session_id = format!("task_{}", Uuid::new_v4().simple());
    let mut output = state
        .harness
        .invoke_stream(prompt, &session_id)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let mut buf = String::new();
    loop {
        match output.stream.recv().await {
            Ok(Some(event)) => {
                if let Some(t) = extract_text_delta(&event) {
                    buf.push_str(&t);
                }
            }
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(buf)
}

/// Proactively speak text to a connected device: TTS + push as a full turn
/// (state -> response -> audio frames -> speak_done -> idle).
async fn speak_to_device(state: &Arc<AppState>, device_id: &str, text: &str) {
    let audio = state.tts.synthesize(text).await.unwrap_or_default();
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
