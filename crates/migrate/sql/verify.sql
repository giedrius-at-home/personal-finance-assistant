-- Per-account posting counts and exact sums, computed by MariaDB, to compare
-- against what was imported.
SELECT JSON_OBJECT('legacy_id', tr.account_id, 'currency', c.code, 'postings', COUNT(*),
    'sum', CAST(SUM(tr.amount) AS CHAR))
FROM transactions tr
JOIN transaction_journals j ON j.id = tr.transaction_journal_id AND j.deleted_at IS NULL
JOIN transaction_currencies c ON c.id = tr.transaction_currency_id
WHERE tr.deleted_at IS NULL
GROUP BY tr.account_id, c.code;
