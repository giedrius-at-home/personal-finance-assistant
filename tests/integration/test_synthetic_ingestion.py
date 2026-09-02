"""End-to-end derived-state proof using invented identities and balances only."""

from datetime import UTC, datetime
from pathlib import Path

import pytest

from finance_assistant.domain.canonical import SourceSystem
from finance_assistant.ingestion.mapping import AccountLink
from finance_assistant.ingestion.service import IngestionService
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyCurrency,
    FireflyPiggyBank,
    FireflyTransaction,
)
from finance_assistant.integrations.ghostfolio import GhostfolioAccount
from finance_assistant.persistence.sqlite import SQLiteDerivedState

SYNTHETIC_FIXTURE_NOTICE = "Invented test data; never copied from a financial system."
NOW = datetime(2026, 1, 2, tzinfo=UTC)


@pytest.mark.anyio
async def test_synthetic_sources_map_persist_and_reconcile_exactly(
    tmp_path: Path,
) -> None:
    currency = FireflyCurrency.model_validate(
        {
            "type": "currencies",
            "id": "synthetic-eur",
            "attributes": {
                "code": "EUR",
                "name": "Synthetic euro",
                "symbol": "SYN",
                "decimal_places": 2,
            },
        }
    )
    firefly_accounts = tuple(
        FireflyAccount.model_validate(
            {
                "type": "accounts",
                "id": account_id,
                "attributes": {
                    "name": f"Synthetic {account_id}",
                    "type": "asset",
                    "currency_code": "EUR",
                    "currency_decimal_places": 2,
                    "current_balance": balance,
                    "updated_at": NOW.isoformat(),
                },
            }
        )
        for account_id, balance in (
            ("synthetic-cash", "1000.01"),
            ("synthetic-broker", "250.02"),
        )
    )
    ghostfolio_account = GhostfolioAccount.model_validate(
        {
            "id": "synthetic-portfolio",
            "name": "Synthetic portfolio mirror",
            "currency": "EUR",
            "balance": "250.02",
            "value": "500.04",
            "valueInBaseCurrency": "500.04",
            "createdAt": NOW.isoformat(),
            "updatedAt": NOW.isoformat(),
        }
    )
    transfer = FireflyTransaction.model_validate(
        {
            "type": "transactions",
            "id": "synthetic-group",
            "attributes": {
                "updated_at": NOW.isoformat(),
                "transactions": [
                    {
                        "transaction_journal_id": "synthetic-transfer",
                        "type": "Transfer",
                        "date": NOW.isoformat(),
                        "currency_code": "EUR",
                        "currency_decimal_places": 2,
                        "amount": "50.03",
                        "description": "Synthetic investment transfer",
                        "source_id": "synthetic-cash",
                        "destination_id": "synthetic-broker",
                    }
                ],
            },
        }
    )

    class SyntheticFirefly:
        async def list_currencies(self) -> tuple[FireflyCurrency, ...]:
            return (currency,)

        async def list_accounts(self) -> tuple[FireflyAccount, ...]:
            return firefly_accounts

        async def list_transactions(self) -> tuple[FireflyTransaction, ...]:
            return (transfer,)

        async def list_piggy_banks(self) -> tuple[FireflyPiggyBank, ...]:
            return ()

    class SyntheticGhostfolio:
        async def list_accounts(self) -> tuple[GhostfolioAccount, ...]:
            return (ghostfolio_account,)

    state = SQLiteDerivedState(tmp_path / "synthetic.sqlite3")
    state.migrate_up()
    service = IngestionService(
        firefly=SyntheticFirefly(),
        ghostfolio=SyntheticGhostfolio(),
        state=state,
        links=(
            AccountLink(
                "synthetic-broker",
                "synthetic-portfolio",
                owner=SourceSystem.GHOSTFOLIO,
            ),
        ),
        clock=lambda: NOW,
    )

    assert SYNTHETIC_FIXTURE_NOTICE.startswith("Invented")
    first = await service.ingest()
    replay = await service.ingest()

    assert first.reconciliation.matched
    assert first.projections_changed == 3
    assert first.goals_seen == 0
    assert replay.projections_changed == 0
    assert len(state.list_accounts()) == 2
    assert len(state.list_transactions()) == 1
    assert (
        state.list_transactions()[0].counterparty_account_id
        == "ghostfolio:synthetic-portfolio"
    )
    state.close()
