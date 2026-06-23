-- Growth-driven content: prompts in PG, level/bond gated unlocks.

-- Global, editable common prompt templates (e.g. common_rules).
CREATE TABLE IF NOT EXISTS prompt_templates (
    key        text PRIMARY KEY,
    content    text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Tiered prompt fragments unlocked by bond/level (the "spirit" layer).
CREATE TABLE IF NOT EXISTS prompt_fragments (
    id        bigserial PRIMARY KEY,
    scope     text NOT NULL DEFAULT 'global',  -- 'global' | a specific actor_id
    kind      text NOT NULL,                   -- 'persona' | 'ability' | 'topic'
    min_bond  int  NOT NULL DEFAULT 0,
    min_level int  NOT NULL DEFAULT 0,
    content   text NOT NULL,
    ord       int  NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_fragments_scope ON prompt_fragments(scope);

-- Wearable/scene catalog with unlock thresholds (the "visual" layer).
CREATE TABLE IF NOT EXISTS catalog_items (
    id           bigserial PRIMARY KEY,
    gender       text NOT NULL,
    slot         text NOT NULL,   -- hair_back|hair_front|costume|blush|accessory|bg
    variant      text NOT NULL,
    min_level    int  NOT NULL DEFAULT 1,
    min_bond     int  NOT NULL DEFAULT 0,
    display_name text,
    UNIQUE(gender, slot, variant)
);
CREATE INDEX IF NOT EXISTS idx_catalog_gender ON catalog_items(gender);
