use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::tasks::{Schedule, ScheduledTask, TaskAction};
use crate::workflows::Workflow;
use crate::ws::AppState;

// ===== device registration & self-service =====

#[derive(Deserialize)]
pub struct RegisterReq {
    pub device_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub persona: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterResp {
    pub api_key: String,
    pub actor_id: String,
    pub name: String,
    pub gender: String,
}

/// POST /register {device_id, name?, gender?, persona?} -> { api_key, actor_id }
/// Dev: open registration. The api_key is shown ONCE; set it on the device.
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterReq>,
) -> Result<(StatusCode, Json<RegisterResp>), (StatusCode, String)> {
    let name = req.name.unwrap_or_else(|| "小爪".to_string());
    let gender = req.gender.unwrap_or_else(|| "girl".to_string());
    let persona = req.persona.unwrap_or_default();
    let (actor, api_key) = crate::db::register(&state.db, &req.device_id, &name, &gender, &persona)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    tracing::info!(actor = %actor.actor_id, device = %req.device_id, "actor registered");
    Ok((
        StatusCode::CREATED,
        Json(RegisterResp { api_key, actor_id: actor.actor_id, name: actor.name, gender: actor.gender }),
    ))
}

#[derive(Deserialize)]
pub struct UpdateMeReq {
    #[serde(default)]
    pub persona: Option<String>,
}

/// PUT /me  (Bearer api_key) — owner updates their own actor's persona.
pub async fn update_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpdateMeReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = crate::auth::authenticate(&headers, &state.db)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    if let Some(p) = req.persona {
        crate::db::update_persona(&state.db, &actor.actor_id, &p)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /session/reset (Bearer api_key) — start a fresh conversation session.
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = crate::auth::authenticate(&headers, &state.db)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    crate::db::new_session(&state.db, &actor.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

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
