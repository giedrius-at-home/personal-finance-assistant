-- Alerts and the daily spending email. The four alert tables are the Python
-- gateway's (migration 3), created here only for a database that never had
-- them; existing rows carry on unchanged.
CREATE TABLE IF NOT EXISTS alert_lifecycle (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    severity TEXT NOT NULL,
    last_fingerprint TEXT NOT NULL,
    cooldown_until TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(subject_kind, subject_id)
);
CREATE TABLE IF NOT EXISTS alert_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    severity TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    reasons_json TEXT NOT NULL,
    disposition TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notification_attempts (
    attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    outcome TEXT NOT NULL,
    attempted_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS evaluation_schedule (
    job_name TEXT PRIMARY KEY,
    last_success_at TEXT,
    lease_until TEXT
);
-- The spending email settings (JSON `EmailSettings`; the SMTP password is in
-- the Keychain, never here).
CREATE TABLE IF NOT EXISTS email_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    payload_json TEXT NOT NULL
);
-- What the Python service kept in `.spending-email.json` and
-- `.spending-digest.json` next to its database: the last send of each kind.
CREATE TABLE IF NOT EXISTS email_receipts (
    kind TEXT PRIMARY KEY CHECK(kind IN ('alert', 'digest')),
    sent_at TEXT NOT NULL,
    severity TEXT
);
