-- Phone notifications, the daily overspend summary and adaptive bank sync.

-- One row per device that turned on push in the installed web app.
CREATE TABLE IF NOT EXISTS push_subscriptions (
    endpoint TEXT PRIMARY KEY,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    origin TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- The owner's "avoidable" tick per spending group; a group without a row
-- takes the default from its name (eating out, delivery, shopping, ...).
CREATE TABLE IF NOT EXISTS avoidable_groups (
    spending_group TEXT PRIMARY KEY,
    avoidable INTEGER NOT NULL CHECK(avoidable IN (0, 1))
);

-- When a bank sync first saw each outflow: banks give dates only, so the
-- time of a purchase lies between the bank's previous sync (`since`) and
-- `seen_at`, unless its description carried the time (`exact_at`).
CREATE TABLE IF NOT EXISTS purchase_sightings (
    entry_key TEXT PRIMARY KEY,
    bank TEXT NOT NULL,
    account_uid TEXT NOT NULL,
    amount TEXT NOT NULL,
    pending INTEGER NOT NULL CHECK(pending IN (0, 1)),
    booked_on TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    since TEXT,
    exact_at TEXT
);
CREATE INDEX IF NOT EXISTS purchase_sightings_seen ON purchase_sightings(seen_at);
