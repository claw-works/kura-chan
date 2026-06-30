use std::sync::Arc;

use axum::extract::State;
use axum::routing::{delete, get, put};
use axum::{Json, Router};

use crate::admin;
use crate::api;
use crate::ws::{self, AppState};

pub fn create_router(state: Arc<AppState>) -> Router {
    // token-gated admin API (X-Admin-Token == ADMIN_TOKEN)
    let admin_api: Router<Arc<AppState>> = Router::new()
        .route("/fragments", get(admin::list_fragments).post(admin::upsert_fragment))
        .route("/fragments/{id}", delete(admin::delete_fragment))
        .route("/catalog", get(admin::list_catalog))
        .route("/catalog/{id}", put(admin::update_catalog))
        .route("/templates", get(admin::list_templates).post(admin::set_template))
        .route("/actors", get(admin::list_actors))
        .route("/actors/{id}", put(admin::set_actor))
        .layer(axum::middleware::from_fn(admin::require_token));

    Router::new()
        .route("/ws/device", get(ws::ws_upgrade))
        .route("/health", get(health))
        .route("/register", axum::routing::post(api::register))
        .route("/me", axum::routing::put(api::update_me))
        .route("/session/reset", axum::routing::post(api::reset_session))
        .route("/history", get(api::get_history))
        .route("/assets/{gender}", get(crate::assets::list_assets))
        .route("/assets/composite/{gender}", get(crate::assets::get_composite))
        .route("/assets/composite_png/{gender}", get(crate::assets::get_composite_png))
        .route("/assets/face/{gender}/{expr}", get(crate::assets::get_face))
        .route("/assets/{gender}/{file}", get(crate::assets::get_asset))
        .route("/tasks", get(api::list_tasks).post(api::create_task))
        .route("/tasks/{id}", delete(api::delete_task))
        .route("/workflows", get(api::list_workflows).post(api::upsert_workflow))
        .route("/workflows/{name}", delete(api::delete_workflow))
        .route("/ui/admin", get(admin::page))
        .nest("/api/admin", admin_api)
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let llm = &state.config.llm;
    Json(serde_json::json!({
        "status": "ok",
        "llm": {
            "format": llm.format,
            "model": llm.model,
            "thinking": llm.thinking,
            "max_tokens": llm.max_tokens,
        }
    }))
}
