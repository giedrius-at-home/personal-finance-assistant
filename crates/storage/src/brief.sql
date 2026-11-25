-- The daily voice brief (Gisel or Rafael).

-- The owner's settings (JSON `BriefSettings`).
CREATE TABLE IF NOT EXISTS brief_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    payload_json TEXT NOT NULL
);

-- One brief per local day: what Gisel said, the facts it came from and the
-- spoken MP3 (NULL when speech failed; the text still went out). The last
-- two weeks are kept.
CREATE TABLE IF NOT EXISTS voice_briefs (
    brief_date TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    title TEXT NOT NULL,
    script TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('mistral', 'fixed')),
    facts_json TEXT NOT NULL,
    audio BLOB,
    -- The voice that spoke it (a mood version when the day called for one).
    voice_id TEXT,
    -- Who spoke it: Gisel or Rafael.
    persona TEXT
);
