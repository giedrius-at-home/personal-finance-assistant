-- Test fixture for the Transactions tab writes: the gateway tables the
-- snapshot reads (as the Python service created them) plus a tiny ledger.
CREATE TABLE owner_settings (singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    content_hash TEXT NOT NULL, payload_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE canonical_goals (goal_id TEXT PRIMARY KEY, source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL, external_id TEXT NOT NULL, content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE spending_group_history (category TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0),
    group_name TEXT, reviewed_at TEXT NOT NULL, PRIMARY KEY(category, revision));
CREATE TABLE custom_spending_groups (name_key TEXT PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE monthly_budget_plans (month TEXT NOT NULL, currency_code TEXT NOT NULL,
    revision INTEGER NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
    PRIMARY KEY(month, currency_code));
CREATE TABLE planned_payments (plan_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL, archived_at TEXT);
CREATE TABLE spending_group_limits (group_key TEXT NOT NULL, currency_code TEXT NOT NULL,
    revision INTEGER NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
    PRIMARY KEY (group_key, currency_code));
CREATE TABLE subscription_category_rules (bill_id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
    updated_at TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE declined_bill_matches (bill_id TEXT NOT NULL, transaction_id TEXT NOT NULL,
    updated_at TEXT NOT NULL, PRIMARY KEY (bill_id, transaction_id));
CREATE TABLE review_deferrals (source_key TEXT PRIMARY KEY, updated_at TEXT NOT NULL);
CREATE TABLE account_roles (account_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE month_periods (month TEXT PRIMARY KEY, revision INTEGER NOT NULL,
    updated_at TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE pots (pot_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE pot_entries (entry_id TEXT PRIMARY KEY, pot_id TEXT NOT NULL,
    month TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE source_versions (source_system TEXT NOT NULL, source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL, content_hash TEXT NOT NULL, source_revision TEXT,
    source_updated_at TEXT, observed_at TEXT NOT NULL, record_kind TEXT NOT NULL,
    canonical_id TEXT NOT NULL, payload_json TEXT NOT NULL,
    PRIMARY KEY (source_system, source_entity, external_id, content_hash));
CREATE TABLE daily_review_policy (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), payload_json TEXT NOT NULL);
CREATE TABLE daily_review_baseline (transaction_id TEXT PRIMARY KEY);
CREATE TABLE daily_review_entries (transaction_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE firefly_sync_health (singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    last_success TEXT, last_attempt TEXT NOT NULL, failed INTEGER NOT NULL, last_full_success TEXT);
CREATE TABLE transaction_details (source_key TEXT PRIMARY KEY, revision INTEGER NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE category_proposals (proposal_id TEXT PRIMARY KEY, source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL, external_id TEXT NOT NULL, feature_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'approved', 'corrected', 'rejected', 'reversed', 'superseded')),
    batch_id TEXT, payload_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE category_corrections (correction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_identity TEXT NOT NULL, feature_hash TEXT NOT NULL, normalized_merchant TEXT NOT NULL,
    approved_category TEXT NOT NULL, approved_at TEXT NOT NULL,
    UNIQUE(source_identity, feature_hash, approved_category));
CREATE TABLE category_batches (batch_id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK(status IN ('approved', 'reversed')), created_at TEXT NOT NULL);

INSERT INTO daily_review_policy VALUES (1,
    '{"activated_at":"2026-10-01T00:00:00Z","timezone":"Europe/Amsterdam","auto_threshold":"0.90","missed_after_days":30}');

-- Account 5, Pocket: an own account with no postings, the target of the move
-- tests; it shows as Unclassified in fixture reviews until a test gives it a role.
INSERT INTO accounts (id, name, kind) VALUES (1, 'Checking', 'asset'), (2, 'Lidl', 'expense'),
    (3, 'Daiva Mirkliene', 'revenue'), (4, 'Albert Heijn', 'expense'),
    (5, 'Pocket', 'asset');
INSERT INTO categories (id, name) VALUES (1, 'Groceries'), (2, 'Eating out');
INSERT INTO transactions (id, kind, date, booked_at, description, source_account_id,
    destination_account_id, category_id, tags) VALUES
    (10, 'withdrawal', '2026-10-20', '2026-10-20T12:00:00', 'Lidl Utrecht', 1, 2, NULL, '["booked"]'),
    (11, 'deposit', '2026-10-21', '2026-10-21T12:00:00', 'Transfer from Daiva Mirkliene', 3, 1, NULL, '[]'),
    (12, 'withdrawal', '2026-10-22', '2026-10-22T12:00:00', 'AH Utrecht', 1, 4, 1, '[]'),
    (13, 'withdrawal', '2026-10-23', '2026-10-23T12:00:00', 'AH Centraal', 1, 4, 1, '[]');
INSERT INTO postings (transaction_id, account_id, amount, currency) VALUES
    (10, 1, '-20.50', 'EUR'), (10, 2, '20.50', 'EUR'),
    (11, 3, '-30.00', 'EUR'), (11, 1, '30.00', 'EUR'),
    (12, 1, '-5.00', 'EUR'), (12, 4, '5.00', 'EUR'),
    (13, 1, '-7.00', 'EUR'), (13, 4, '7.00', 'EUR');
INSERT INTO transactions_search (rowid, text) VALUES (10, 'Lidl Utrecht'), (11, 'Transfer from Daiva Mirkliene'),
    (12, 'AH Utrecht'), (13, 'AH Centraal');
