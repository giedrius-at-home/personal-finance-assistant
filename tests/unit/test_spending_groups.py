import asyncio
from datetime import UTC, date, datetime
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from finance_assistant.api import create_app
from finance_assistant.categorisation_runtime import CategorySuggestionSettings
from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.domain.spending_groups import (
    GroupMapping,
    SpendingGroup,
    monthly_spending,
)
from finance_assistant.integrations.ollama import OllamaClient, OllamaProtocolError
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)
from finance_assistant.spending_group_suggestions import suggest_spending_group

NOW = datetime(2026, 9, 9, 12, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def transaction(
    key: str,
    amount: str = "10",
    *,
    category: str | None = "Synthetic shop",
    kind: TransactionKind = TransactionKind.OUTFLOW,
    when: datetime = NOW,
    currency: Currency = EUR,
) -> CanonicalTransaction:
    return CanonicalTransaction(
        transaction_id=key,
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=key,
        ),
        account_id="synthetic-a",
        occurred_at=when,
        kind=kind,
        counterparty_account_id="synthetic-b"
        if kind is TransactionKind.TRANSFER
        else None,
        amount=Money(amount=amount, currency=currency),
        category=category,
        description="Fictitious test purchase",
    )


def test_monthly_totals_confirmed_categories_refunds_and_separate_currencies() -> None:
    rows = (
        transaction("a", "30"),
        transaction("b", "5", kind=TransactionKind.REFUND),
        transaction("c", "20", category=None),
        transaction("d", "100", kind=TransactionKind.TRANSFER),
        transaction("e", "1000", kind=TransactionKind.INFLOW),
        transaction("f", "40", when=NOW.replace(month=8)),
        transaction("future", "999", when=NOW.replace(hour=13)),
        transaction("usd", "7", currency=Currency(code="USD", minor_unit=2)),
    )
    report = monthly_spending(
        rows,
        (
            GroupMapping(
                category="Synthetic shop", group=SpendingGroup.GROCERIES, revision=1
            ),
        ),
        as_of=NOW,
        timezone="Europe/Amsterdam",
    )
    euro, usd = report.currencies
    assert euro.currency == EUR
    assert euro.spending == 45 and euro.gross == 50 and euro.refunds == 5
    assert euro.previous_spending == 40 and euro.comparison_spending == 45
    assert euro.transfer_count == 1
    assert sum(group.spending for group in euro.groups) == euro.spending
    assert sum(group.share_percent for group in euro.groups) == 100
    grocery = next(
        group for group in euro.groups if group.group == SpendingGroup.GROCERIES
    )
    assert grocery.spending == 25 and grocery.share_percent == 60
    assert grocery.stores[0].category == "Synthetic shop"
    assert usd.previous_spending is None and usd.spending == 7
    assert len(report.mappings) == 1


def test_short_previous_month_timezone_unknown_and_refund_only() -> None:
    now = datetime(2026, 3, 31, 12, tzinfo=UTC)
    report = monthly_spending(
        (
            transaction("a", "4", when=now, category="Uncategorised"),
            transaction(
                "b",
                "3",
                when=now.replace(day=28),
                kind=TransactionKind.REFUND,
                category=None,
            ),
            transaction("c", "2", when=now.replace(month=2, day=28)),
        ),
        (),
        as_of=now,
        timezone="Europe/Amsterdam",
    )
    assert report.comparison_through == date(2026, 3, 28)
    assert report.previous_through == date(2026, 2, 28)
    assert report.currencies[0].spending == 1
    assert report.currencies[0].comparison_spending == -3
    assert {
        store.category
        for group in report.currencies[0].groups
        for store in group.stores
    } == {None, "Uncategorised", "Synthetic shop"}
    assert monthly_spending((), (), as_of=now, timezone="UTC").currencies == ()
    with pytest.raises(ValueError, match="timezone-aware"):
        monthly_spending((), (), as_of=now.replace(tzinfo=None), timezone="UTC")


def test_mapping_revision_survives_restart_unassign_and_prevents_stale_save(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    mapping = GroupMapping(
        category="Synthetic shop", group=SpendingGroup.GROCERIES, revision=0
    )
    state.save_group_mapping(mapping, now=NOW)
    assert state.list_group_mappings()[0].revision == 1
    with pytest.raises(PersistenceConflictError):
        state.save_group_mapping(mapping, now=NOW)
    state.save_group_mapping(
        mapping.model_copy(update={"group": None, "revision": 1}), now=NOW
    )
    state.close()
    state = SQLiteDerivedState(path)
    state.migrate_up()
    assert state.list_group_mappings()[0].group is None
    assert state.list_group_mappings()[0].revision == 2
    with pytest.raises(PersistenceConflictError):
        state.migrate_down()
    with pytest.raises(ValueError):
        state.save_group_mapping(mapping, now=NOW.replace(tzinfo=None))
    state.close()


def test_month_boundary_source_filter_and_zero_purchase_share() -> None:
    now = datetime(2026, 1, 1, 0, 30, tzinfo=UTC)
    external = transaction("external").model_copy(
        update={
            "source": SourceReference(
                system=SourceSystem.GHOSTFOLIO,
                entity=SourceEntity.TRANSACTION,
                external_id="x",
            )
        }
    )
    report = monthly_spending(
        (
            external,
            transaction("refund", "2", kind=TransactionKind.REFUND, when=now),
            transaction("previous", "8", when=datetime(2025, 12, 1, tzinfo=UTC)),
            transaction(
                "local-new-month", "3", when=datetime(2025, 12, 31, 23, 30, tzinfo=UTC)
            ),
        ),
        (),
        as_of=now,
        timezone="Europe/Amsterdam",
    )
    assert report.previous_start == date(2025, 12, 1)
    assert report.currencies[0].spending == 1
    refund_only = monthly_spending(
        (transaction("refund", "2", kind=TransactionKind.REFUND),),
        (),
        as_of=NOW,
        timezone="UTC",
    )
    assert refund_only.currencies[0].groups[0].share_percent == 0
    assert refund_only.currencies[0].spending == -2


@pytest.mark.parametrize(
    "response",
    [
        '{"group":"Groceries"}',
        '{"group":null}',
        '{"group":"Invented"}',
        '{"group":"Groceries", "extra":1}',
        "x" * 513,
    ],
)
def test_bounded_model_suggestions_abstain_and_validate(
    monkeypatch: pytest.MonkeyPatch,
    response: str,
) -> None:
    async def generate(
        self: OllamaClient, prompt: str, *, system: str | None = None
    ) -> str:
        assert "Synthetic shop" in prompt and "untrusted data" in (system or "")
        assert "transaction" not in prompt and "balance" not in prompt
        return response

    monkeypatch.setattr(OllamaClient, "generate", generate)

    async def run() -> None:
        async with OllamaClient(
            base_url="http://localhost:11434", model="synthetic"
        ) as client:
            if response in ('{"group":"Groceries"}', '{"group":null}'):
                result = await suggest_spending_group("Synthetic shop", (), client)
                assert result.group in (None, SpendingGroup.GROCERIES)
            else:
                with pytest.raises(OllamaProtocolError):
                    await suggest_spending_group("Synthetic shop", (), client)

    asyncio.run(run())


def test_model_route_never_saves_mapping_and_failure_is_safe(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "model.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    state.upsert(transaction("a"), observed_at=NOW)
    state.close()
    reply = '{"group":"Groceries"}'

    async def generate(
        self: OllamaClient, prompt: str, *, system: str | None = None
    ) -> str:
        return reply

    monkeypatch.setattr(OllamaClient, "generate", generate)
    with TestClient(
        create_app(
            database_path=path,
            category_suggestion_settings=CategorySuggestionSettings(
                base_url="http://localhost:11434", model="synthetic"
            ),
        )
    ) as client:
        page = client.get("/dashboard")
        assert "Ask local model" in page.text
        form = {"csrf_token": page.cookies["pfa_csrf"], "category": "Synthetic shop"}
        headers = {"origin": "http://testserver"}
        assert client.post("/spending-groups/suggest", data=form).status_code == 403
        assert client.post(
            "/spending-groups/suggest", data=form, headers=headers
        ).json() == {"group": "Groceries"}
        assert (
            client.post(
                "/spending-groups/suggest",
                data={**form, "category": "Absent"},
                headers=headers,
            ).status_code
            == 422
        )
        reply = "invalid"
        assert (
            client.post(
                "/spending-groups/suggest", data=form, headers=headers
            ).status_code
            == 503
        )
    state = SQLiteDerivedState(path)
    assert state.list_group_mappings() == ()
    state.close()


def test_disabled_group_routes_and_empty_migration() -> None:
    with TestClient(create_app(demo_mode=True)) as client:
        page = client.get("/dashboard")
        form = {"csrf_token": page.cookies["pfa_csrf"]}
        for route in ("save", "suggest"):
            assert (
                client.post(
                    f"/spending-groups/{route}",
                    data=form,
                    headers={"origin": "http://testserver"},
                ).status_code
                == 409
            )
    state = SQLiteDerivedState(":memory:")
    state.migrate_up()
    state.migrate_down()
    state.migrate_up()
    assert state.list_group_mappings() == ()
    state.close()


def test_reviewed_mapping_route_csrf_conflict_validation_and_no_source_mutation(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    source = transaction("a")
    state.upsert(source, observed_at=NOW)
    state.close()
    with TestClient(create_app(database_path=path)) as client:
        page = client.get("/dashboard")
        assert "Monthly spending" in page.text and "Needs a group" in page.text
        form = {
            "csrf_token": page.cookies["pfa_csrf"],
            "category": "Synthetic shop",
            "group": "Groceries",
            "revision": "0",
        }
        assert client.post("/spending-groups/save", data=form).status_code == 403
        headers = {"origin": "http://testserver"}
        saved = client.post("/spending-groups/save", data=form, headers=headers)
        assert saved.status_code == 200 and "Spending group saved" in saved.text
        assert (
            client.post("/spending-groups/save", data=form, headers=headers).status_code
            == 409
        )
        assert (
            client.post(
                "/spending-groups/save",
                data={**form, "group": "Invalid"},
                headers=headers,
            ).status_code
            == 422
        )
        assert (
            client.post(
                "/spending-groups/save",
                data={**form, "category": "Absent"},
                headers=headers,
            ).status_code
            == 422
        )
    state = SQLiteDerivedState(path)
    assert state.list_transactions() == (source,)
    state.close()
