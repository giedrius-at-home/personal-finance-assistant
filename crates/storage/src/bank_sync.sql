-- Enable Banking sync: the outcome of the last attempt per bank (JSON
-- `BankRun`), so the scheduler keeps its daily cadence across launches.
CREATE TABLE IF NOT EXISTS bank_sync_runs (
    bank TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);
