-- AI suggestions: the owner's settings (JSON `AiSettings`; secrets stay in
-- the Keychain) and the public research cache (Python `research_cache.py`,
-- formerly its own SQLite file).
CREATE TABLE IF NOT EXISTS ai_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS research_cache (
    query_key TEXT PRIMARY KEY,
    purpose TEXT NOT NULL,
    result_json TEXT NOT NULL,
    retrieved_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
