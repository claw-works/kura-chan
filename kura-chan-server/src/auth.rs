use axum::http::HeaderMap;

use crate::db::{self, Actor, Db};

/// Authenticate via `Authorization: Bearer <api_key>` against the actors table.
/// Returns the matching actor (api key == actor == character).
pub async fn authenticate(headers: &HeaderMap, db: &Db) -> Result<Actor, String> {
    let key = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| "Missing/invalid Authorization header".to_string())?;
    db::actor_by_key(db, key)
        .await
        .ok_or_else(|| "Invalid API key".to_string())
}
