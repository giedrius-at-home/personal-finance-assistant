-- Ledger tables that replace Firefly III. Ids are the Firefly ids for rows
-- migrated from it, so references in the gateway tables keep resolving.
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY,
    legacy_firefly_id INTEGER UNIQUE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    iban TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    role TEXT,
    include_net_worth INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY,
    legacy_firefly_id INTEGER UNIQUE,
    name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS bills (
    id INTEGER PRIMARY KEY,
    legacy_firefly_id INTEGER UNIQUE,
    name TEXT NOT NULL,
    currency TEXT NOT NULL,
    amount_min TEXT NOT NULL,
    amount_max TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT,
    repeat_freq TEXT NOT NULL,
    skip INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY,
    legacy_firefly_id INTEGER UNIQUE,
    group_id INTEGER,
    kind TEXT NOT NULL,
    date TEXT NOT NULL,
    booked_at TEXT NOT NULL,
    description TEXT NOT NULL,
    source_account_id INTEGER NOT NULL REFERENCES accounts(id),
    destination_account_id INTEGER NOT NULL REFERENCES accounts(id),
    category_id INTEGER REFERENCES categories(id),
    bill_id INTEGER REFERENCES bills(id),
    external_id TEXT,
    internal_reference TEXT,
    notes TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    reconciled INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS transactions_date ON transactions(date DESC, id DESC);
CREATE INDEX IF NOT EXISTS transactions_category ON transactions(category_id);
CREATE INDEX IF NOT EXISTS transactions_source ON transactions(source_account_id);
CREATE INDEX IF NOT EXISTS transactions_destination ON transactions(destination_account_id);

-- One signed row per account touched by a transaction (double entry).
CREATE TABLE IF NOT EXISTS postings (
    id INTEGER PRIMARY KEY,
    legacy_firefly_id INTEGER UNIQUE,
    transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    foreign_amount TEXT,
    foreign_currency TEXT
);
CREATE INDEX IF NOT EXISTS postings_account ON postings(account_id);
CREATE INDEX IF NOT EXISTS postings_transaction ON postings(transaction_id);

-- Substring search over description, notes, tags and counterparties.
CREATE VIRTUAL TABLE IF NOT EXISTS transactions_search USING fts5(
    text, content='', contentless_delete=1, tokenize='trigram'
);

CREATE TABLE IF NOT EXISTS import_runs (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    finished_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    stats TEXT NOT NULL
);
