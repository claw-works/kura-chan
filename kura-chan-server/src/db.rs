use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub type Db = PgPool;

/// One registered actor = api key = character (1:1).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Actor {
    pub actor_id: String,
    pub device_id: Option<String>,
    pub name: String,
    pub gender: String,
    pub persona: String,
    pub level: i32,
    pub xp: i32,
    pub bond: i32,
    pub energy: i32,
    pub appearance: serde_json::Value,
}

pub fn hash_key(key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    hex::encode(h.finalize())
}

fn rand_hex(n: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

pub async fn connect(url: &str) -> Result<Db, sqlx::Error> {
    let pool = PgPoolOptions::new().max_connections(8).connect(url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

const ACTOR_COLS: &str = "actor_id, device_id, name, gender, persona, level, xp, bond, energy, appearance";

/// XP required to clear `level` (per-level, superlinear): xp_base * L * (L+1).
/// e.g. base=50 -> L1:100, L2:300, L3:600, L4:1000 — higher levels need more.
pub fn xp_need(level: i32, xp_base: i32) -> i32 {
    let l = level.max(1) as i64;
    (xp_base as i64 * l * (l + 1)) as i32
}

pub async fn actor_by_key(db: &Db, api_key: &str) -> Option<Actor> {
    let h = hash_key(api_key);
    sqlx::query_as::<_, Actor>(&format!(
        "SELECT {ACTOR_COLS} FROM actors WHERE api_key_hash = $1"
    ))
    .bind(h)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

pub async fn actor_by_id(db: &Db, actor_id: &str) -> Option<Actor> {
    sqlx::query_as::<_, Actor>(&format!(
        "SELECT {ACTOR_COLS} FROM actors WHERE actor_id = $1"
    ))
    .bind(actor_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

pub async fn actor_by_device(db: &Db, device_id: &str) -> Option<Actor> {
    sqlx::query_as::<_, Actor>(&format!(
        "SELECT {ACTOR_COLS} FROM actors WHERE device_id = $1 ORDER BY created_at LIMIT 1"
    ))
    .bind(device_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Register a new actor. Returns (actor, plaintext api_key shown once).
pub async fn register(
    db: &Db,
    device_id: &str,
    name: &str,
    gender: &str,
    persona: &str,
) -> Result<(Actor, String), sqlx::Error> {
    let actor_id = format!("actor_{}", rand_hex(8));
    let api_key = format!("kc_{}", rand_hex(20));
    sqlx::query(
        "INSERT INTO actors (actor_id, api_key_hash, device_id, name, gender, persona) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&actor_id)
    .bind(hash_key(&api_key))
    .bind(device_id)
    .bind(name)
    .bind(gender)
    .bind(persona)
    .execute(db)
    .await?;
    let actor = actor_by_id(db, &actor_id).await.ok_or(sqlx::Error::RowNotFound)?;
    Ok((actor, api_key))
}

/// Seed a fixed dev actor (api key dev_key_001) if it doesn't exist, so the
/// current device keeps working before real registration.
pub async fn seed_dev(db: &Db) -> Result<(), sqlx::Error> {
    let h = hash_key("dev_key_001");
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT actor_id FROM actors WHERE api_key_hash = $1")
            .bind(&h)
            .fetch_optional(db)
            .await?;
    if exists.is_none() {
        sqlx::query(
            "INSERT INTO actors (actor_id, api_key_hash, device_id, name, gender) \
             VALUES ('actor_seed_dev', $1, 'SEED_DEV', '小爪', 'girl')",
        )
        .bind(&h)
        .execute(db)
        .await?;
        tracing::info!("seeded dev actor (dev_key_001 -> actor_mixue)");
    }
    Ok(())
}

pub async fn update_persona(db: &Db, actor_id: &str, persona: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE actors SET persona=$2, updated_at=now() WHERE actor_id=$1")
        .bind(actor_id)
        .bind(persona)
        .execute(db)
        .await?;
    Ok(())
}

// ---- sessions ----

/// Get the actor's active session if recent (within ttl_secs), else open a new one.
pub async fn get_or_create_session(
    db: &Db,
    actor_id: &str,
    ttl_secs: i64,
) -> Result<String, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT session_id FROM sessions \
         WHERE actor_id=$1 AND active \
           AND last_activity > now() - ($2 || ' seconds')::interval \
         ORDER BY last_activity DESC LIMIT 1",
    )
    .bind(actor_id)
    .bind(ttl_secs.to_string())
    .fetch_optional(db)
    .await?;
    if let Some((sid,)) = row {
        return Ok(sid);
    }
    new_session(db, actor_id).await
}

/// Deactivate the actor's sessions and open a fresh one.
pub async fn new_session(db: &Db, actor_id: &str) -> Result<String, sqlx::Error> {
    sqlx::query("UPDATE sessions SET active=false WHERE actor_id=$1 AND active")
        .bind(actor_id)
        .execute(db)
        .await?;
    let sid = format!("ses_{}", rand_hex(16)); // 4+32=36 chars (AgentCore requires >=33)
    sqlx::query("INSERT INTO sessions (session_id, actor_id) VALUES ($1,$2)")
        .bind(&sid)
        .bind(actor_id)
        .execute(db)
        .await?;
    Ok(sid)
}

pub async fn touch_session(db: &Db, session_id: &str) {
    let _ = sqlx::query("UPDATE sessions SET last_activity=now() WHERE session_id=$1")
        .bind(session_id)
        .execute(db)
        .await;
}

// ---- messages ----

// ---- growth / appearance (server-authoritative) ----

/// Apply growth deltas; returns the updated actor (with level-ups applied).
/// Energy first regenerates passively based on real time since the last update
/// (`regen_per_hour`), then `denergy` is applied. Call with all-zero deltas to
/// just settle the passive energy regen (e.g. on connect).
pub async fn bump_growth(
    db: &Db,
    actor_id: &str,
    dxp: i32,
    dbond: i32,
    denergy: i32,
    xp_base: i32,
    regen_per_hour: i32,
) -> Option<Actor> {
    let mut a = actor_by_id(db, actor_id).await?;
    // passive energy regen since last update
    let elapsed_secs: f64 = sqlx::query_scalar(
        "SELECT extract(epoch from now() - updated_at)::float8 FROM actors WHERE actor_id=$1",
    )
    .bind(actor_id)
    .fetch_one(db)
    .await
    .unwrap_or(0.0);
    let regen = (elapsed_secs / 3600.0 * regen_per_hour as f64) as i32;

    a.xp += dxp;
    while a.xp >= xp_need(a.level, xp_base) {
        a.xp -= xp_need(a.level, xp_base);
        a.level += 1;
    }
    a.bond = (a.bond + dbond).clamp(0, 100);
    a.energy = (a.energy + regen + denergy).clamp(0, 100);
    let _ = sqlx::query(
        "UPDATE actors SET level=$2, xp=$3, bond=$4, energy=$5, updated_at=now() WHERE actor_id=$1",
    )
    .bind(actor_id)
    .bind(a.level)
    .bind(a.xp)
    .bind(a.bond)
    .bind(a.energy)
    .execute(db)
    .await;
    Some(a)
}

/// Merge the device-reported appearance selection into the stored jsonb.
pub async fn set_appearance(db: &Db, actor_id: &str, appearance: &serde_json::Value) {
    let _ = sqlx::query("UPDATE actors SET appearance = appearance || $2, updated_at=now() WHERE actor_id=$1")
        .bind(actor_id)
        .bind(appearance)
        .execute(db)
        .await;
}

/// Merge a single appearance key (server-owned, e.g. bg).
pub async fn set_appearance_key(db: &Db, actor_id: &str, key: &str, value: serde_json::Value) {
    let obj = serde_json::json!({ key: value });
    let _ = sqlx::query("UPDATE actors SET appearance = appearance || $2, updated_at=now() WHERE actor_id=$1")
        .bind(actor_id)
        .bind(obj)
        .execute(db)
        .await;
}

pub async fn log_message(db: &Db, session_id: &str, actor_id: &str, role: &str, content: &str) {
    let _ = sqlx::query(
        "INSERT INTO messages (session_id, actor_id, role, content) VALUES ($1,$2,$3,$4)",
    )
    .bind(session_id)
    .bind(actor_id)
    .bind(role)
    .bind(content)
    .execute(db)
    .await;
}
