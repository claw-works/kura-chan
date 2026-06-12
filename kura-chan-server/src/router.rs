use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::ws::{self, AppState};

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws/device", get(ws::ws_upgrade))
        .route("/health", get(health))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
