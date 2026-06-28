-- DB-backed scheduled jobs (cron). Replaces the old file-backed task store.
CREATE TABLE IF NOT EXISTS jobs (
    id          text PRIMARY KEY,
    actor_id    text NOT NULL REFERENCES actors(actor_id) ON DELETE CASCADE,
    device_id   text NOT NULL,
    label       text NOT NULL DEFAULT '',     -- short human name for voice reference
    action      jsonb NOT NULL,               -- {type: say|agent_prompt|workflow, ...}
    schedule    jsonb NOT NULL,               -- {type: once|interval|daily, ...}
    enabled     boolean NOT NULL DEFAULT true,
    next_fire   bigint NOT NULL,              -- unix seconds
    created_at  bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_due ON jobs(enabled, next_fire);
CREATE INDEX IF NOT EXISTS idx_jobs_actor ON jobs(actor_id);
