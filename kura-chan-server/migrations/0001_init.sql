-- kura-chan multi-tenant schema
CREATE TABLE IF NOT EXISTS actors (
    actor_id     text PRIMARY KEY,
    api_key_hash text UNIQUE NOT NULL,
    device_id    text,
    name         text NOT NULL DEFAULT '小爪',
    gender       text NOT NULL DEFAULT 'girl',   -- girl / boy (appearance set)
    persona      text NOT NULL DEFAULT '',       -- personality prefix prepended to the common rules
    appearance   jsonb NOT NULL DEFAULT '{}'::jsonb, -- {hair, costume, blush, glasses, ...}
    level        int  NOT NULL DEFAULT 1,
    xp           int  NOT NULL DEFAULT 0,
    bond         int  NOT NULL DEFAULT 20,
    energy       int  NOT NULL DEFAULT 100,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id    text PRIMARY KEY,
    actor_id      text NOT NULL REFERENCES actors(actor_id) ON DELETE CASCADE,
    started_at    timestamptz NOT NULL DEFAULT now(),
    last_activity timestamptz NOT NULL DEFAULT now(),
    active        boolean NOT NULL DEFAULT true
);
CREATE INDEX IF NOT EXISTS idx_sessions_actor_active ON sessions(actor_id, active);

CREATE TABLE IF NOT EXISTS messages (
    id         bigserial PRIMARY KEY,
    session_id text NOT NULL,
    actor_id   text NOT NULL,
    role       text NOT NULL,   -- 'user' | 'assistant'
    content    text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);
