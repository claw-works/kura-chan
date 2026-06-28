-- Per-actor TTS voice, format "provider/voiceid" (e.g. volc/zh_female_...).
ALTER TABLE actors
    ADD COLUMN IF NOT EXISTS voice text NOT NULL
    DEFAULT 'volc/zh_female_sajiaoxuemei_uranus_bigtts';
