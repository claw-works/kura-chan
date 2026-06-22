use std::sync::Arc;

use axum::routing::{delete, get};
use axum::Router;

use crate::api;
use crate::ws::{self, AppState};

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws/device", get(ws::ws_upgrade))
        .route("/health", get(health))
        .route("/register", axum::routing::post(api::register))
        .route("/me", axum::routing::put(api::update_me))
        .route("/session/reset", axum::routing::post(api::reset_session))
        .route("/assets/{gender}", get(crate::assets::list_assets))
        .route("/assets/composite/{gender}", get(crate::assets::get_composite))
        .route("/assets/face/{gender}/{expr}", get(crate::assets::get_face))
        .route("/assets/{gender}/{file}", get(crate::assets::get_asset))
        .route("/tasks", get(api::list_tasks).post(api::create_task))
        .route("/tasks/{id}", delete(api::delete_task))
        .route("/workflows", get(api::list_workflows).post(api::upsert_workflow))
        .route("/workflows/{name}", delete(api::delete_workflow))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
