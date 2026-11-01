-- Repair pocket transfers the bank sync booked as purchases (migration 6).
-- Idempotent: every statement selects only rows still in the wrong shape.
--
-- Only rows booked from 1 November 2026 on are touched: the ledger was
-- reconciled to the banks at the October 2026 closure, so older mis-booked
-- pocket legs are settled history whose balances a repair would break.
--
-- Expects a temporary table `pocket_routes (via, target)` with the savings
-- routes: `target` is an own account whose role is `savings`, reached via
-- the own account `via` (`account_roles` is a gateway table that a fresh
-- store does not have, so `Store` fills it before running this file).

DROP TABLE IF EXISTS temp.pocket_pairs;
DROP TABLE IF EXISTS temp.pocket_savings;
DROP TABLE IF EXISTS temp.pocket_repaired;

CREATE TEMP TABLE pocket_repaired (id INTEGER PRIMARY KEY);

-- Rule 2: a transfer between two own accounts of one consent arrived as a
-- withdrawal to the unknown-destination account and a deposit from the
-- unknown-source account (the bank named no counterparty), both with the
-- bank's reference, on the same day, for the same amount.
CREATE TEMP TABLE pocket_pairs AS
SELECT w.id AS withdrawal,
       d.id AS deposit,
       d.destination_account_id AS destination
FROM transactions w
JOIN transactions d
  ON d.kind = 'deposit'
 AND d.external_id = w.external_id
 AND d.date = w.date
 AND d.destination_account_id != w.source_account_id
JOIN postings pw ON pw.transaction_id = w.id AND pw.account_id = w.source_account_id
JOIN postings pd ON pd.transaction_id = d.id AND pd.account_id = d.destination_account_id
JOIN accounts ws ON ws.id = w.source_account_id AND ws.kind = 'asset'
JOIN accounts dd ON dd.id = d.destination_account_id AND dd.kind = 'asset'
JOIN accounts wd ON wd.id = w.destination_account_id AND wd.kind = 'expense' AND wd.name = '(unknown destination)'
JOIN accounts ds ON ds.id = d.source_account_id AND ds.kind = 'revenue' AND ds.name = '(unknown source)'
WHERE w.kind = 'withdrawal'
  AND w.date >= '2026-11-01'
  AND w.external_id IS NOT NULL
  AND w.external_id != ''
  AND pw.currency = pd.currency
  AND -CAST(pw.amount AS REAL) = CAST(pd.amount AS REAL);

-- One deposit per withdrawal and one withdrawal per deposit.
DELETE FROM pocket_pairs
WHERE deposit != (SELECT min(deposit) FROM pocket_pairs p WHERE p.withdrawal = pocket_pairs.withdrawal);
DELETE FROM pocket_pairs
WHERE withdrawal != (SELECT min(withdrawal) FROM pocket_pairs p WHERE p.deposit = pocket_pairs.deposit);

-- The withdrawal becomes the transfer: its expense posting now belongs to
-- the account the money entered.
UPDATE postings
SET account_id = (SELECT destination FROM pocket_pairs p WHERE p.withdrawal = postings.transaction_id)
WHERE transaction_id IN (SELECT withdrawal FROM pocket_pairs)
  AND account_id = (SELECT destination_account_id FROM transactions t WHERE t.id = postings.transaction_id);

UPDATE transactions
SET kind = 'transfer',
    destination_account_id = (SELECT destination FROM pocket_pairs p WHERE p.withdrawal = transactions.id),
    category_id = NULL
WHERE id IN (SELECT withdrawal FROM pocket_pairs);

INSERT OR IGNORE INTO pocket_repaired (id) SELECT withdrawal FROM pocket_pairs;

-- The deposit was the same movement seen from the other side.
DELETE FROM transactions_search WHERE rowid IN (SELECT deposit FROM pocket_pairs);
DELETE FROM postings WHERE transaction_id IN (SELECT deposit FROM pocket_pairs);
DELETE FROM transactions WHERE id IN (SELECT deposit FROM pocket_pairs);

-- Rule 3: "To <savings pocket>" / "From <savings pocket>" with no known
-- counterparty is a transfer to or from the one savings account routed via
-- this account.
CREATE TEMP TABLE pocket_savings AS
SELECT t.id AS id,
       t.kind AS kind,
       r.target AS target
FROM transactions t
JOIN accounts own
  ON own.id = CASE t.kind WHEN 'withdrawal' THEN t.source_account_id ELSE t.destination_account_id END
 AND own.kind = 'asset'
JOIN accounts cp
  ON cp.id = CASE t.kind WHEN 'withdrawal' THEN t.destination_account_id ELSE t.source_account_id END
JOIN pocket_routes r ON r.via = own.id AND r.target != own.id
JOIN accounts target ON target.id = r.target AND target.kind = 'asset' AND target.active = 1
WHERE t.date >= '2026-11-01'
  AND ((t.kind = 'withdrawal' AND cp.kind = 'expense' AND cp.name = '(unknown destination)'
        AND lower(t.description) LIKE 'to %')
    OR (t.kind = 'deposit' AND cp.kind = 'revenue' AND cp.name = '(unknown source)'
        AND lower(t.description) LIKE 'from %'))
  AND (lower(t.description) LIKE '%saving%'
    OR lower(t.description) LIKE '%vault%'
    OR lower(t.description) LIKE '%pocket%'
    OR lower(t.description) LIKE '%spaar%')
  AND (SELECT count(*) FROM pocket_routes r2 WHERE r2.via = own.id) = 1;

UPDATE postings
SET account_id = (SELECT target FROM pocket_savings s WHERE s.id = postings.transaction_id)
WHERE transaction_id IN (SELECT id FROM pocket_savings)
  AND account_id = (SELECT CASE kind WHEN 'withdrawal' THEN destination_account_id ELSE source_account_id END
                    FROM transactions t WHERE t.id = postings.transaction_id);

UPDATE transactions
SET kind = 'transfer',
    destination_account_id = (SELECT target FROM pocket_savings s WHERE s.id = transactions.id),
    category_id = NULL
WHERE id IN (SELECT id FROM pocket_savings WHERE kind = 'withdrawal');

UPDATE transactions
SET kind = 'transfer',
    source_account_id = (SELECT target FROM pocket_savings s WHERE s.id = transactions.id),
    category_id = NULL
WHERE id IN (SELECT id FROM pocket_savings WHERE kind = 'deposit');

INSERT OR IGNORE INTO pocket_repaired (id) SELECT id FROM pocket_savings;

-- Search rows of the repaired transactions: description, both account
-- names, notes and tags, joined as `search_text` does (no category now).
DELETE FROM transactions_search WHERE rowid IN (SELECT id FROM pocket_repaired);
INSERT INTO transactions_search (rowid, text)
SELECT t.id,
       t.description
       || ' • ' || s.name
       || ' • ' || d.name
       || CASE WHEN t.notes IS NULL OR t.notes = '' THEN '' ELSE ' • ' || t.notes END
       || coalesce((SELECT ' • ' || group_concat(value, ' • ') FROM json_each(t.tags)), '')
FROM transactions t
JOIN accounts s ON s.id = t.source_account_id
JOIN accounts d ON d.id = t.destination_account_id
WHERE t.id IN (SELECT id FROM pocket_repaired);

DROP TABLE temp.pocket_pairs;
DROP TABLE temp.pocket_savings;
DROP TABLE temp.pocket_repaired;
