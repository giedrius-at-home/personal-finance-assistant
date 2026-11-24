CREATE TABLE IF NOT EXISTS ui_layout (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), payload_json TEXT NOT NULL);
