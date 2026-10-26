-- Firefly III ledger as JSON lines, one object per row, tagged by "t".
-- Soft-deleted rows are skipped; amounts are cast to text to stay exact.
SELECT JSON_OBJECT('t', 'account', 'id', a.id, 'name', a.name,
    'kind', CASE t.type
        WHEN 'Asset account' THEN 'asset' WHEN 'Default account' THEN 'asset'
        WHEN 'Cash account' THEN 'cash'
        WHEN 'Expense account' THEN 'expense' WHEN 'Beneficiary account' THEN 'expense'
        WHEN 'Revenue account' THEN 'revenue'
        WHEN 'Loan' THEN 'liability' WHEN 'Debt' THEN 'liability' WHEN 'Mortgage' THEN 'liability'
        WHEN 'Credit card' THEN 'liability' WHEN 'Liability credit account' THEN 'liability'
        WHEN 'Initial balance account' THEN 'opening'
        WHEN 'Reconciliation account' THEN 'reconciliation'
        ELSE 'other' END,
    'iban', NULLIF(a.iban, ''), 'active', a.active = 1 AND a.deleted_at IS NULL,
    'role', (SELECT JSON_UNQUOTE(m.data) FROM account_meta m WHERE m.account_id = a.id AND m.name = 'account_role' LIMIT 1),
    'include_net_worth', COALESCE((SELECT JSON_UNQUOTE(m.data) FROM account_meta m
        WHERE m.account_id = a.id AND m.name = 'include_net_worth' LIMIT 1), '1') IN ('1', 'true'))
FROM accounts a JOIN account_types t ON t.id = a.account_type_id
-- Deleted accounts are kept (inactive) when live postings still point at them.
WHERE a.deleted_at IS NULL OR EXISTS (SELECT 1 FROM transactions tr WHERE tr.account_id = a.id AND tr.deleted_at IS NULL);

SELECT JSON_OBJECT('t', 'category', 'id', id, 'name', name) FROM categories WHERE deleted_at IS NULL;

SELECT JSON_OBJECT('t', 'bill', 'id', b.id, 'name', b.name, 'currency', c.code,
    'amount_min', CAST(b.amount_min AS CHAR), 'amount_max', CAST(b.amount_max AS CHAR),
    'start_date', DATE_FORMAT(b.date, '%Y-%m-%d'), 'end_date', DATE_FORMAT(b.end_date, '%Y-%m-%d'),
    'repeat_freq', b.repeat_freq, 'skip', b.skip, 'active', b.active = 1)
FROM bills b JOIN transaction_currencies c ON c.id = b.transaction_currency_id
WHERE b.deleted_at IS NULL;

SELECT JSON_OBJECT('t', 'transaction', 'id', j.id, 'group_id', j.transaction_group_id,
    'kind', CASE tt.type
        WHEN 'Withdrawal' THEN 'withdrawal' WHEN 'Deposit' THEN 'deposit' WHEN 'Transfer' THEN 'transfer'
        WHEN 'Opening balance' THEN 'opening' WHEN 'Reconciliation' THEN 'reconciliation'
        ELSE 'other' END,
    'booked_at', DATE_FORMAT(j.date, '%Y-%m-%dT%H:%i:%s'),
    'description', j.description,
    'category_id', (SELECT MIN(cj.category_id) FROM category_transaction_journal cj
        JOIN categories c ON c.id = cj.category_id AND c.deleted_at IS NULL
        WHERE cj.transaction_journal_id = j.id),
    'bill_id', (SELECT b.id FROM bills b WHERE b.id = j.bill_id AND b.deleted_at IS NULL),
    'external_id', (SELECT JSON_UNQUOTE(m.data) FROM journal_meta m
        WHERE m.transaction_journal_id = j.id AND m.name = 'external_id' AND m.deleted_at IS NULL LIMIT 1),
    'internal_reference', (SELECT JSON_UNQUOTE(m.data) FROM journal_meta m
        WHERE m.transaction_journal_id = j.id AND m.name = 'internal_reference' AND m.deleted_at IS NULL LIMIT 1),
    'notes', (SELECT n.text FROM notes n WHERE n.noteable_id = j.id AND n.deleted_at IS NULL
        AND n.noteable_type LIKE '%TransactionJournal' AND n.text <> '' LIMIT 1),
    'tags', COALESCE((SELECT JSON_ARRAYAGG(tg.tag) FROM tag_transaction_journal tj
        JOIN tags tg ON tg.id = tj.tag_id AND tg.deleted_at IS NULL WHERE tj.transaction_journal_id = j.id),
        JSON_ARRAY()))
FROM transaction_journals j JOIN transaction_types tt ON tt.id = j.transaction_type_id
WHERE j.deleted_at IS NULL;

SELECT JSON_OBJECT('t', 'posting', 'id', tr.id, 'transaction_id', tr.transaction_journal_id,
    'account_id', tr.account_id, 'amount', CAST(tr.amount AS CHAR), 'currency', c.code,
    'foreign_amount', CAST(tr.foreign_amount AS CHAR), 'foreign_currency', fc.code,
    'reconciled', tr.reconciled = 1)
FROM transactions tr
JOIN transaction_journals j ON j.id = tr.transaction_journal_id AND j.deleted_at IS NULL
JOIN transaction_currencies c ON c.id = tr.transaction_currency_id
LEFT JOIN transaction_currencies fc ON fc.id = tr.foreign_currency_id
WHERE tr.deleted_at IS NULL;
