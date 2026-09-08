from __future__ import annotations

from datetime import UTC, date, datetime, timedelta
from decimal import Decimal
from types import TracebackType
from typing import Self

import pytest

from finance_assistant.csv_import import (
    CsvImportConflictError,
    CsvImportCoordinator,
    CsvImportError,
    CsvRowStatus,
    ParsedCsvRow,
    csv_external_id,
    detect_bank_label,
    parse_bank_csv,
)
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyAccountAttributes,
    FireflyTransaction,
    FireflyTransactionAttributes,
    FireflyTransactionSplit,
)


def account(
    account_id: str = "asset-1", name: str = "Synthetic bank"
) -> FireflyAccount:
    return FireflyAccount(
        type="accounts",
        id=account_id,
        attributes=FireflyAccountAttributes(
            name=name,
            type="asset",
            currency_code="EUR",
            currency_decimal_places=2,
            current_balance="100",
        ),
    )


def transaction(
    description: str,
    *,
    amount: str = "12.34",
    external_id: str | None = None,
) -> FireflyTransaction:
    return FireflyTransaction(
        type="transactions",
        id=f"group-{description}",
        attributes=FireflyTransactionAttributes(
            transactions=(
                FireflyTransactionSplit(
                    transaction_journal_id=f"journal-{description}",
                    type="withdrawal",
                    date=datetime(2026, 9, 1, tzinfo=UTC),
                    currency_code="EUR",
                    currency_decimal_places=2,
                    amount=amount,
                    description=description,
                    source_id="asset-1",
                    destination_id="expense-1",
                    external_id=external_id,
                ),
            )
        ),
    )


class FakeClient:
    def __init__(
        self,
        *,
        accounts: tuple[FireflyAccount, ...],
        transactions: tuple[FireflyTransaction, ...] = (),
    ) -> None:
        self.accounts = accounts
        self.transactions = transactions
        self.writes: list[tuple[ParsedCsvRow, str]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        return None

    async def list_accounts(self) -> tuple[FireflyAccount, ...]:
        return self.accounts

    async def list_transactions(
        self, *, start: date | None = None, end: date | None = None
    ) -> tuple[FireflyTransaction, ...]:
        return self.transactions

    async def create_csv_transaction(
        self,
        *,
        row: ParsedCsvRow,
        account: FireflyAccount,
        accounts: tuple[FireflyAccount, ...],
        external_id: str,
    ) -> None:
        assert account in accounts
        self.writes.append((row, external_id))


@pytest.mark.parametrize(
    ("filename", "payload", "expected_amounts"),
    [
        (
            "abn-amro.csv",
            "Transaction Date;Transaction Amount;Currency Code;Description;Reference\n"
            "01-09-2026;-12,34;EUR;Synthetic shop;abn-1\n",
            (Decimal("-12.34"),),
        ),
        (
            "revolut.csv",
            "Completed Date,Amount,Currency,Description,Transaction ID\n"
            "2026-09-01T12:00:00Z,25.00,EUR,Synthetic transfer,rev-1\n",
            (Decimal("25.00"),),
        ),
        (
            "swedbank.csv",
            "Date;Debit;Credit;Currency;Details;Reference\n"
            "2026-09-01;5,10;;EUR;Synthetic debit;swe-1\n"
            "2026-09-02;;9,20;EUR;Synthetic credit;swe-2\n",
            (Decimal("-5.10"), Decimal("9.20")),
        ),
        (
            "trade-republic.txt",
            "Date\tValue\tCurrency\tDescription\tReference\n"
            "2026/09/01\t1.234,56\tEUR\tSynthetic activity\ttr-1\n",
            (Decimal("1234.56"),),
        ),
    ],
)
def test_provider_neutral_parser_handles_common_exports(
    filename: str, payload: str, expected_amounts: tuple[Decimal, ...]
) -> None:
    rows, rejected = parse_bank_csv(payload.encode(), filename=filename)

    assert tuple(row.amount for row in rows) == expected_amounts
    assert rejected == 0
    assert detect_bank_label(filename, rows) != "Connected bank CSV"


def test_parser_rejects_unsafe_or_unmappable_files() -> None:
    with pytest.raises(CsvImportError, match="non-empty"):
        parse_bank_csv(b"", filename="empty.csv")
    with pytest.raises(CsvImportError, match="CSV or delimited"):
        parse_bank_csv(b"date,amount,description\n", filename="export.xlsx")
    with pytest.raises(CsvImportError, match="date and amount"):
        parse_bank_csv(b"unknown,description\n1,Synthetic\n", filename="bank.csv")
    with pytest.raises(CsvImportError, match="description"):
        parse_bank_csv(b"date,amount\n2026-09-01,1\n", filename="bank.csv")
    with pytest.raises(CsvImportError, match="no valid"):
        parse_bank_csv(
            b"date,amount,description\ninvalid,0,Synthetic\n", filename="bank.csv"
        )


@pytest.mark.anyio
async def test_preview_classifies_existing_possible_and_new_rows() -> None:
    client = FakeClient(
        accounts=(account(),),
        transactions=(
            transaction("Synthetic exact"),
            transaction("Synthetic other", amount="50.00"),
        ),
    )
    coordinator = CsvImportCoordinator(client_factory=lambda: client)
    payload = (
        b"Date,Amount,Currency,Description,Reference\n"
        b"2026-09-01,-12.34,EUR,Synthetic exact,ref-1\n"
        b"2026-09-01,-50.00,EUR,Synthetic ambiguous,ref-2\n"
        b"2026-09-02,-7.00,EUR,Synthetic new,ref-3\n"
    )

    preview = await coordinator.preview(
        content=payload, filename="abn-amro.csv", account_id="asset-1"
    )

    assert tuple(row.status for row in preview.rows) == (
        CsvRowStatus.DUPLICATE,
        CsvRowStatus.POSSIBLE_DUPLICATE,
        CsvRowStatus.NEW,
    )
    assert not preview.committable
    with pytest.raises(CsvImportConflictError, match="possible duplicate"):
        await coordinator.commit(preview.preview_id)


@pytest.mark.anyio
async def test_exact_preview_commits_only_new_rows_once() -> None:
    existing_row = ParsedCsvRow(
        line_number=2,
        booked_on=date(2026, 9, 1),
        amount="-12.34",
        currency="EUR",
        description="Synthetic exact",
        external_reference="existing-ref",
    )
    existing_external_id = csv_external_id(
        existing_row, account_id="asset-1", bank_label="ABN AMRO"
    )
    client = FakeClient(
        accounts=(account(),),
        transactions=(
            transaction("Synthetic exact", external_id=existing_external_id),
        ),
    )
    coordinator = CsvImportCoordinator(
        client_factory=lambda: client,
        clock=lambda: datetime(2026, 9, 8, tzinfo=UTC),
    )
    payload = (
        b"Date,Amount,Currency,Description,Reference\n"
        b"2026-09-01,-12.34,EUR,Synthetic exact,existing-ref\n"
        b"2026-09-02,-7.00,EUR,Synthetic new,new-ref\n"
    )
    preview = await coordinator.preview(
        content=payload, filename="abn-amro.csv", account_id="asset-1"
    )

    assert preview.committable
    assert await coordinator.commit(preview.preview_id) == 1
    assert len(client.writes) == 1
    assert client.writes[0][0].description == "Synthetic new"
    assert client.writes[0][1].startswith("pfa-csv-v1-")
    with pytest.raises(CsvImportConflictError, match="missing or expired"):
        await coordinator.commit(preview.preview_id)


@pytest.mark.anyio
async def test_identical_rows_without_provider_reference_require_attention() -> None:
    payload = (
        b"Date,Amount,Currency,Description\n"
        b"2026-09-01,-1.00,EUR,Synthetic\n"
        b"2026-09-01,-1.00,EUR,Synthetic\n"
    )
    client = FakeClient(accounts=(account(),))
    coordinator = CsvImportCoordinator(client_factory=lambda: client)

    preview = await coordinator.preview(
        content=payload, filename="bank.csv", account_id="asset-1"
    )

    assert tuple(row.status for row in preview.rows) == (
        CsvRowStatus.NEW,
        CsvRowStatus.POSSIBLE_DUPLICATE,
    )
    assert not preview.committable


@pytest.mark.anyio
async def test_existing_matches_are_consumed_once_by_direction() -> None:
    client = FakeClient(
        accounts=(account(),),
        transactions=(transaction("Synthetic repeated", amount="4.00"),),
    )
    coordinator = CsvImportCoordinator(client_factory=lambda: client)
    payload = (
        b"Date,Amount,Currency,Description\n"
        b"2026-09-01,-4.00,EUR,Synthetic repeated\n"
        b"2026-09-01,-4.00,EUR,Synthetic repeated\n"
        b"2026-09-01,4.00,EUR,Synthetic repeated\n"
    )

    preview = await coordinator.preview(
        content=payload, filename="bank.csv", account_id="asset-1"
    )

    assert tuple(row.status for row in preview.rows) == (
        CsvRowStatus.DUPLICATE,
        CsvRowStatus.POSSIBLE_DUPLICATE,
        CsvRowStatus.NEW,
    )


@pytest.mark.anyio
async def test_rejected_rows_currency_mismatch_and_expired_preview_block_commit() -> (
    None
):
    current = datetime(2026, 9, 8, tzinfo=UTC)
    client = FakeClient(accounts=(account(),))
    coordinator = CsvImportCoordinator(
        client_factory=lambda: client, clock=lambda: current
    )
    preview = await coordinator.preview(
        content=(
            b"Date,Amount,Currency,Description\n"
            b"invalid,-1.00,EUR,Synthetic invalid\n"
            b"2026-09-01,-2.00,EUR,Synthetic valid\n"
        ),
        filename="bank.csv",
        account_id="asset-1",
    )
    assert preview.rejected_count == 1
    with pytest.raises(CsvImportConflictError, match="rejected"):
        await coordinator.commit(preview.preview_id)

    with pytest.raises(CsvImportConflictError, match="currency"):
        await coordinator.preview(
            content=(
                b"Date,Amount,Currency,Description\n"
                b"2026-09-01,-2.00,USD,Synthetic mismatch\n"
            ),
            filename="bank.csv",
            account_id="asset-1",
        )

    expiring = CsvImportCoordinator(
        client_factory=lambda: client,
        clock=lambda: current,
    )
    fresh = await expiring.preview(
        content=(
            b"Date,Amount,Currency,Description,Reference\n"
            b"2026-09-01,-2.00,EUR,Synthetic expiring,unique-1\n"
        ),
        filename="bank.csv",
        account_id="asset-1",
    )
    expiring._clock = lambda: current + timedelta(minutes=31)  # noqa: SLF001
    with pytest.raises(CsvImportConflictError, match="missing or expired"):
        await expiring.commit(fresh.preview_id)
