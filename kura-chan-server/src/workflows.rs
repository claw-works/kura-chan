use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// A reusable, customizable "class of request" run through the harness. The
/// prompt_template may contain `{param}` placeholders filled from a task's
/// params. Tools/data-collection happen on the harness side; this is just the
/// request the server sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub prompt_template: String,
}

impl Workflow {
    /// Fill `{key}` placeholders with params (unknown placeholders left as-is).
    pub fn render(&self, params: &BTreeMap<String, String>) -> String {
        let mut out = self.prompt_template.clone();
        for (k, v) in params {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }
}

fn defaults() -> Vec<Workflow> {
    vec![
        Workflow {
            name: "weather_report".into(),
            description: "播报某地天气".into(),
            prompt_template: "查询{city}今天的天气，用小爪的口吻一句话播报（含温度和天气状况）。".into(),
        },
        Workflow {
            name: "daily_briefing".into(),
            description: "每日简报".into(),
            prompt_template: "用小爪的口吻，一两句话跟主人道早安并简单提醒今天要打起精神。".into(),
        },
    ]
}

/// File-backed workflow registry (JSON). Editable via HTTP API; swap for DB later.
pub struct WorkflowStore {
    path: PathBuf,
    workflows: Mutex<Vec<Workflow>>,
}

impl WorkflowStore {
    pub fn load(path: PathBuf) -> Self {
        let workflows = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Workflow>>(&s).ok())
            .unwrap_or_else(|| {
                let d = defaults();
                tracing::info!("no workflows file; seeding {} defaults", d.len());
                d
            });
        let store = Self { path, workflows: Mutex::new(workflows) };
        store.persist(&store.workflows.lock().unwrap());
        store
    }

    fn persist(&self, workflows: &[Workflow]) {
        if let Ok(json) = serde_json::to_string_pretty(workflows) {
            let _ = std::fs::write(&self.path, json);
        }
    }

    pub fn get(&self, name: &str) -> Option<Workflow> {
        self.workflows.lock().unwrap().iter().find(|w| w.name == name).cloned()
    }

    pub fn list(&self) -> Vec<Workflow> {
        self.workflows.lock().unwrap().clone()
    }

    /// Add or replace a workflow by name.
    pub fn upsert(&self, wf: Workflow) {
        let mut guard = self.workflows.lock().unwrap();
        guard.retain(|w| w.name != wf.name);
        guard.push(wf);
        self.persist(&guard);
    }

    pub fn remove(&self, name: &str) -> bool {
        let mut guard = self.workflows.lock().unwrap();
        let before = guard.len();
        guard.retain(|w| w.name != name);
        let changed = guard.len() != before;
        if changed {
            self.persist(&guard);
        }
        changed
    }
}
