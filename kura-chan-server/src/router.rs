use std::sync::Arc;

use axum::routing::{delete, get};
use axum::Router;

use crate::api;
use crate::ws::{self, AppState};

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws/device", get(ws::ws_upgrade))
        .route("/health", get(health))
        .route("/tasks", get(api::list_tasks).post(api::create_task))
        .route("/tasks/:id", delete(api::delete_task))
        .route("/workflows", get(api::list_workflows).post(api::upsert_workflow))
        .route("/workflows/:name", delete(api::delete_workflow))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
