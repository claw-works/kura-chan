use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// When a task should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Fire once at this unix timestamp (seconds), then disable.
    Once { at: i64 },
    /// Fire every `secs` seconds, starting `secs` from creation.
    Interval { secs: u64 },
    /// Fire every day at this Beijing (UTC+8) wall-clock time.
    Daily { hour: u32, minute: u32 },
}

/// Next unix-second for a daily Beijing-time hh:mm (today if still ahead, else tomorrow).
fn next_daily(hour: u32, minute: u32) -> i64 {
    let bj = now_unix() + 8 * 3600;
    let day = bj.div_euclid(86400);
    let target = day * 86400 + (hour as i64) * 3600 + (minute as i64) * 60;
    let next_bj = if target > bj { target } else { target + 86400 };
    next_bj - 8 * 3600
}

/// What to do when the task fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskAction {
    /// Speak this text to the device verbatim (TTS + push).
    Say { text: String },
    /// Run this ad-hoc prompt through the agent (harness), then speak the reply.
    AgentPrompt { prompt: String },
    /// Run a named, reusable workflow (prompt template + params) through the
    /// agent. Tools/data-collection live on the harness side; the server just
    /// renders the request, runs it, and pushes the spoken result.
    Workflow {
        name: String,
        #[serde(default)]
        params: std::collections::BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub device_id: String,
    pub action: TaskAction,
    pub schedule: Schedule,
    pub enabled: bool,
    /// Next fire time (unix seconds).
    pub next_fire: i64,
    pub created_at: i64,
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl ScheduledTask {
    pub fn new(device_id: String, action: TaskAction, schedule: Schedule) -> Self {
        let now = now_unix();
        let next_fire = match &schedule {
            Schedule::Once { at } => *at,
            Schedule::Interval { secs } => now + *secs as i64,
            Schedule::Daily { hour, minute } => next_daily(*hour, *minute),
        };
        Self {
            id: format!("task_{}", Uuid::new_v4().simple()),
            device_id,
            action,
            schedule,
            enabled: true,
            next_fire,
            created_at: now,
        }
    }

    /// Advance after firing. Returns false if the task is done (one-shot).
    fn reschedule(&mut self, now: i64) -> bool {
        match &self.schedule {
            Schedule::Once { .. } => {
                self.enabled = false;
                false
            }
            Schedule::Interval { secs } => {
                self.next_fire = now + *secs as i64;
                true
            }
            Schedule::Daily { hour, minute } => {
                self.next_fire = next_daily(*hour, *minute);
                true
            }
        }
    }
}

/// File-backed task store (JSON). Swap for a DB when moving to SaaS.
pub struct TaskStore {
    path: PathBuf,
    tasks: Mutex<Vec<ScheduledTask>>,
}

impl TaskStore {
    /// Load tasks from `path` (creating an empty store if absent/invalid).
    pub fn load(path: PathBuf) -> Self {
        let tasks = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<ScheduledTask>>(&s).ok())
            .unwrap_or_default();
        tracing::info!(count = tasks.len(), path = %path.display(), "task store loaded");
        Self { path, tasks: Mutex::new(tasks) }
    }

    fn persist(&self, tasks: &[ScheduledTask]) {
        match serde_json::to_string_pretty(tasks) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    tracing::error!(error = %e, "failed to persist tasks");
                }
            }
            Err(e) => tracing::error!(error = %e, "failed to serialize tasks"),
        }
    }

    pub fn add(&self, task: ScheduledTask) -> ScheduledTask {
        let mut guard = self.tasks.lock().unwrap();
        guard.push(task.clone());
        self.persist(&guard);
        task
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut guard = self.tasks.lock().unwrap();
        let before = guard.len();
        guard.retain(|t| t.id != id);
        let changed = guard.len() != before;
        if changed {
            self.persist(&guard);
        }
        changed
    }

    pub fn list(&self) -> Vec<ScheduledTask> {
        self.tasks.lock().unwrap().clone()
    }

    pub fn list_for_device(&self, device_id: &str) -> Vec<ScheduledTask> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.device_id == device_id)
            .cloned()
            .collect()
    }

    /// Take all enabled tasks due at `now`, reschedule/disable them, persist, and
    /// return the fired snapshots for the executor to act on.
    pub fn take_due(&self, now: i64) -> Vec<ScheduledTask> {
        let mut guard = self.tasks.lock().unwrap();
        let mut due = Vec::new();
        for t in guard.iter_mut() {
            if t.enabled && t.next_fire <= now {
                due.push(t.clone());
                t.reschedule(now);
            }
        }
        // drop disabled one-shots
        guard.retain(|t| t.enabled);
        if !due.is_empty() {
            self.persist(&guard);
        }
        due
    }
}
