use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::tasks::{Schedule, ScheduledTask, TaskAction};
use crate::workflows::Workflow;
use crate::ws::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    pub device_id: Option<String>,
}

/// GET /tasks            -> all tasks
/// GET /tasks?device_id= -> tasks for one device
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Json<Vec<ScheduledTask>> {
    let tasks = match q.device_id {
        Some(d) => state.task_store.list_for_device(&d),
        None => state.task_store.list(),
    };
    Json(tasks)
}

#[derive(Deserialize)]
pub struct CreateTaskReq {
    pub device_id: String,
    pub action: TaskAction,
    pub schedule: Schedule,
}

/// POST /tasks  body: { device_id, action:{type:"say",text}|{type:"agent_prompt",prompt},
///                      schedule:{type:"once",at}|{type:"interval",secs} }
pub async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTaskReq>,
) -> (StatusCode, Json<ScheduledTask>) {
    let task = ScheduledTask::new(req.device_id, req.action, req.schedule);
    let saved = state.task_store.add(task);
    tracing::info!(task = %saved.id, device = %saved.device_id, "task created via HTTP");
    (StatusCode::CREATED, Json(saved))
}

/// DELETE /tasks/:id
pub async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.task_store.remove(&id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// GET /workflows -> list reusable request templates
pub async fn list_workflows(State(state): State<Arc<AppState>>) -> Json<Vec<Workflow>> {
    Json(state.workflow_store.list())
}

/// POST /workflows  body: { name, description?, prompt_template }  (upsert by name)
pub async fn upsert_workflow(
    State(state): State<Arc<AppState>>,
    Json(wf): Json<Workflow>,
) -> (StatusCode, Json<Workflow>) {
    state.workflow_store.upsert(wf.clone());
    tracing::info!(workflow = %wf.name, "workflow upserted via HTTP");
    (StatusCode::OK, Json(wf))
}

/// DELETE /workflows/:name
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> StatusCode {
    if state.workflow_store.remove(&name) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
