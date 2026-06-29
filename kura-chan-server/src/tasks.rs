use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// When a task should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Fire once at this unix timestamp (seconds), then disable.
    Once { at: i64 },
    /// Recurring, by a standard 5-field cron expression evaluated in Beijing
    /// (UTC+8) time, e.g. "*/30 9-18 * * 1-5" (every 30min, 9–18h, Mon–Fri).
    Cron { expr: String },
}

/// Next unix-second a cron expression fires strictly after `after_unix`
/// (Beijing time). Returns None if the expression is invalid.
pub fn cron_next(expr: &str, after_unix: i64) -> Option<i64> {
    use chrono::{FixedOffset, TimeZone};
    let cron = croner::Cron::new(expr).parse().ok()?;
    let bj = FixedOffset::east_opt(8 * 3600)?;
    let after = bj.timestamp_opt(after_unix, 0).single()?;
    let next = cron.find_next_occurrence(&after, false).ok()?;
    Some(next.timestamp())
}

/// Whether a cron expression is parseable / schedulable.
pub fn cron_valid(expr: &str) -> bool {
    cron_next(expr, now_unix()).is_some()
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
            Schedule::Cron { expr } => cron_next(expr, now).unwrap_or(i64::MAX),
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

    /// Advance after firing. Returns false if the task is done (one-shot or a
    /// cron expression that no longer yields a next time).
    pub fn reschedule(&mut self, now: i64) -> bool {
        match &self.schedule {
            Schedule::Once { .. } => {
                self.enabled = false;
                false
            }
            Schedule::Cron { expr } => match cron_next(expr, now) {
                Some(next) => {
                    self.next_fire = next;
                    true
                }
                None => {
                    self.enabled = false;
                    false
                }
            },
        }
    }
}
