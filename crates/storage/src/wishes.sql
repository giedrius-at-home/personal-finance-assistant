CREATE TABLE IF NOT EXISTS wishes (wish_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wish_settings (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), payload_json TEXT NOT NULL);
