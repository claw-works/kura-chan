// Temporary admin API for the /ui/admin page. Token-gated via ADMIN_TOKEN env.
// High-privilege (edits prompts/growth) — keep behind a strong token and prefer
// not to expose publicly.

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::db;
use crate::ws::AppState;

fn admin_token() -> Option<String> {
    std::env::var("ADMIN_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Middleware: require a matching `X-Admin-Token` header on every /api/admin route.
pub async fn require_token(headers: HeaderMap, req: Request, next: Next) -> Response {
    let expected = match admin_token() {
        Some(t) => t,
        None => return (StatusCode::FORBIDDEN, "ADMIN_TOKEN not set on server").into_response(),
    };
    let got = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if got != expected {
        return (StatusCode::UNAUTHORIZED, "bad admin token").into_response();
    }
    next.run(req).await
}

/// The admin SPA (no token needed to load the page; the page prompts for it and
/// sends it on every API call).
pub async fn page() -> Html<&'static str> {
    Html(include_str!("admin.html"))
}

// ---- prompt_fragments ----

pub async fn list_fragments(State(s): State<Arc<AppState>>) -> Json<Vec<db::FragmentRow>> {
    Json(db::admin_list_fragments(&s.db).await)
}

#[derive(Deserialize)]
pub struct FragmentReq {
    pub id: Option<i64>,
    pub scope: String,
    pub kind: String,
    pub min_bond: i32,
    pub min_level: i32,
    pub content: String,
    pub ord: i32,
}

pub async fn upsert_fragment(
    State(s): State<Arc<AppState>>,
    Json(r): Json<FragmentReq>,
) -> Response {
    match db::admin_upsert_fragment(
        &s.db, r.id, &r.scope, &r.kind, r.min_bond, r.min_level, &r.content, r.ord,
    )
    .await
    {
        Ok(id) => (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn delete_fragment(State(s): State<Arc<AppState>>, Path(id): Path<i64>) -> StatusCode {
    if db::admin_delete_fragment(&s.db, id).await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- catalog_items ----

pub async fn list_catalog(State(s): State<Arc<AppState>>) -> Json<Vec<db::CatalogRow>> {
    Json(db::admin_list_catalog(&s.db).await)
}

#[derive(Deserialize)]
pub struct CatalogReq {
    pub min_level: i32,
    pub min_bond: i32,
}

pub async fn update_catalog(
    State(s): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(r): Json<CatalogReq>,
) -> StatusCode {
    if db::admin_update_catalog(&s.db, id, r.min_level, r.min_bond).await {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- prompt_templates ----

pub async fn list_templates(State(s): State<Arc<AppState>>) -> Json<Vec<db::TemplateRow>> {
    Json(db::admin_list_templates(&s.db).await)
}

#[derive(Deserialize)]
pub struct TemplateReq {
    pub key: String,
    pub content: String,
}

pub async fn set_template(
    State(s): State<Arc<AppState>>,
    Json(r): Json<TemplateReq>,
) -> StatusCode {
    db::set_prompt_template(&s.db, &r.key, &r.content).await;
    StatusCode::OK
}

// ---- actors (growth) ----

pub async fn list_actors(State(s): State<Arc<AppState>>) -> Json<Vec<db::ActorRow>> {
    Json(db::admin_list_actors(&s.db).await)
}

#[derive(Deserialize)]
pub struct ActorGrowthReq {
    pub level: i32,
    pub bond: i32,
    pub energy: i32,
}

pub async fn set_actor(
    State(s): State<Arc<AppState>>,
    Path(actor_id): Path<String>,
    Json(r): Json<ActorGrowthReq>,
) -> StatusCode {
    if db::admin_set_actor_growth(&s.db, &actor_id, r.level, r.bond, r.energy).await {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
