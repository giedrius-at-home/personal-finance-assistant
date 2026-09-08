import hashlib
from datetime import UTC, date, datetime
from decimal import Decimal
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from finance_assistant.api import _owner_name_aliases, create_app
from finance_assistant.categorisation_service import CategoryProposal, ProposalStatus
from finance_assistant.csv_import import (
    CsvImportConflictError,
    CsvImportError,
    CsvImportPreview,
    CsvPreviewRow,
    CsvRowStatus,
)
from finance_assistant.domain.alerting import AlertPolicy, budget_alert_candidate
from finance_assistant.domain.canonical import (
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.categorisation import CategorisationMethod
from finance_assistant.domain.models import BudgetWindow, Currency, Money
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.web import format_money

NOW = datetime(2026, 9, 2, tzinfo=UTC)
SAME_ORIGIN = {"origin": "http://testserver"}
EUR = Currency(code="EUR", minor_unit=2)


class StubCsvCoordinator:
    def __init__(self) -> None:
        self.preview_calls: list[tuple[bytes, str, str]] = []
        self.commit_calls: list[str] = []

    async def preview(
        self, *, content: bytes, filename: str, account_id: str
    ) -> CsvImportPreview:
        self.preview_calls.append((content, filename, account_id))
        return CsvImportPreview(
            preview_id="p" * 43,
            bank_label="Synthetic bank",
            account_name="Synthetic account",
            row_count=2,
            new_count=1,
            duplicate_count=1,
            possible_duplicate_count=0,
            rejected_count=0,
            rows=(
                CsvPreviewRow(
                    booked_on=date(2026, 9, 1),
                    amount=Decimal("-1.25"),
                    currency="EUR",
                    description="Synthetic merchant",
                    status=CsvRowStatus.NEW,
                ),
            ),
        )

    async def commit(self, preview_id: str) -> int:
        self.commit_calls.append(preview_id)
        return 1


class RaisingCsvCoordinator(StubCsvCoordinator):
    def __init__(
        self,
        *,
        preview_error: Exception | None = None,
        commit_error: Exception | None = None,
    ) -> None:
        super().__init__()
        self.preview_error = preview_error
        self.commit_error = commit_error

    async def preview(
        self, *, content: bytes, filename: str, account_id: str
    ) -> CsvImportPreview:
        if self.preview_error is not None:
            raise self.preview_error
        return await super().preview(
            content=content, filename=filename, account_id=account_id
        )

    async def commit(self, preview_id: str) -> int:
        if self.commit_error is not None:
            raise self.commit_error
        return await super().commit(preview_id)


def test_money_display_uses_currency_precision_without_negative_zero() -> None:
    assert format_money(Decimal("77.470000000000"), EUR) == "77.47"
    assert format_money(Decimal("-0.00"), EUR) == "0.00"


def proposal(name: str) -> CategoryProposal:
    return CategoryProposal(
        proposal_id=hashlib.sha256(name.encode()).hexdigest(),
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=name,
            source_updated_at=NOW,
        ),
        proposed_category="Groceries",
        method=CategorisationMethod.MODEL,
        confidence=Decimal("0.91"),
        runner_up="Dining",
        runner_up_confidence=Decimal("0.05"),
        reasons=("Synthetic evidence",),
        model_version="qwen3:8b",
        prompt_version="test-v1",
        policy_version="test-v1",
        vocabulary_version="test-v1",
        feature_hash=hashlib.sha256(f"feature:{name}".encode()).hexdigest(),
        normalized_merchant=name,
        created_at=NOW,
    )


def test_demo_dashboard_is_invented_responsive_and_no_store() -> None:
    client = TestClient(create_app(demo_mode=True))

    response = client.get("/dashboard")

    assert response.status_code == 200
    assert "Invented demo data" in response.text
    assert "Northstar Bicycle Repair" in response.text
    assert "133.20 spent" in response.text
    assert "2266.80 weekly cash flow · 2400.00 income" in response.text
    assert "10300.00 EUR" in response.text
    assert "Cash accounts" in response.text
    assert "Invented current account" in response.text
    assert "Invented portfolio" in response.text
    assert "writes disabled" in response.text
    assert response.headers["cache-control"] == "no-store"
    assert "frame-ancestors 'none'" in response.headers["content-security-policy"]
    assert response.cookies["pfa_csrf"]


def test_owner_name_alias_environment_is_bounded_and_deduplicated() -> None:
    assert _owner_name_aliases(None) == ()
    assert _owner_name_aliases(" Synthetic Owner, S Owner, Synthetic Owner ") == (
        "Synthetic Owner",
        "S Owner",
    )

    with pytest.raises(ValueError, match="oversized"):
        _owner_name_aliases("x" * 101)


def test_dashboard_json_uses_the_same_bounded_view() -> None:
    response = TestClient(create_app(demo_mode=True)).get("/v1/dashboard")

    assert response.status_code == 200
    payload = response.json()
    assert payload["data_mode"] == "invented demo"
    assert len(payload["review_items"]) == 3
    assert payload["upstream_writes"] == "disabled"


def test_mutation_requires_same_origin_csrf_and_persistent_state(
    tmp_path: Path,
) -> None:
    client = TestClient(create_app(database_path=tmp_path / "state.sqlite3"))
    page = client.get("/dashboard")
    token = page.cookies["pfa_csrf"]

    missing = client.post(
        "/settings/preview", data={"csrf_token": "wrong"}, headers=SAME_ORIGIN
    )
    foreign = client.post(
        "/settings/preview",
        data={"csrf_token": token},
        headers={"origin": "https://attacker.invalid"},
    )
    no_origin = client.post("/settings/preview", data={"csrf_token": token})

    assert missing.status_code == 403
    assert foreign.status_code == 403
    assert no_origin.status_code == 403


def test_csv_upload_requires_preview_and_exact_owner_commit(tmp_path: Path) -> None:
    coordinator = StubCsvCoordinator()
    with TestClient(
        create_app(
            database_path=tmp_path / "state.sqlite3",
            csv_import_coordinator=coordinator,  # type: ignore[arg-type]
        )
    ) as client:
        page = client.get("/dashboard")
        token = page.cookies["pfa_csrf"]
        assert "Upload any connected bank CSV" in page.text
        preview = client.post(
            "/imports/csv/preview",
            data={"csrf_token": token, "account_id": "asset-1"},
            files={"upload": ("synthetic.csv", b"safe synthetic CSV", "text/csv")},
            headers=SAME_ORIGIN,
        )
        assert preview.status_code == 200
        assert "1 new" in preview.text
        assert "duplicates skipped" in preview.text
        assert "safe synthetic CSV" not in preview.text
        committed = client.post(
            "/imports/csv/commit",
            data={"csrf_token": token, "preview_id": "p" * 43},
            headers=SAME_ORIGIN,
            follow_redirects=False,
        )
        assert committed.status_code == 303
        assert committed.headers["location"] == (
            "/dashboard?csv_imported=1&csv_refreshed=false"
        )

    assert coordinator.preview_calls == [
        (b"safe synthetic CSV", "synthetic.csv", "asset-1")
    ]
    assert coordinator.commit_calls == ["p" * 43]


def test_csv_upload_rejects_cross_origin_and_oversized_files(tmp_path: Path) -> None:
    coordinator = StubCsvCoordinator()
    with TestClient(
        create_app(
            database_path=tmp_path / "state.sqlite3",
            csv_import_coordinator=coordinator,  # type: ignore[arg-type]
        )
    ) as client:
        token = client.get("/dashboard").cookies["pfa_csrf"]
        foreign = client.post(
            "/imports/csv/preview",
            data={"csrf_token": token, "account_id": "asset-1"},
            files={"upload": ("synthetic.csv", b"x", "text/csv")},
            headers={"origin": "https://attacker.invalid"},
        )
        oversized = client.post(
            "/imports/csv/preview",
            data={"csrf_token": token, "account_id": "asset-1"},
            files={
                "upload": (
                    "synthetic.csv",
                    b"x" * (5 * 1024 * 1024 + 1),
                    "text/csv",
                )
            },
            headers=SAME_ORIGIN,
        )

    assert foreign.status_code == 403
    assert oversized.status_code == 413


def test_csv_routes_fail_closed_and_render_refresh_status(tmp_path: Path) -> None:
    path = tmp_path / "state.sqlite3"
    with TestClient(create_app(database_path=path)) as disabled:
        token = disabled.get("/dashboard").cookies["pfa_csrf"]
        response = disabled.post(
            "/imports/csv/preview",
            data={"csrf_token": token, "account_id": "auto"},
            files={"upload": ("synthetic.csv", b"x", "text/csv")},
            headers=SAME_ORIGIN,
        )
        assert response.status_code == 409

    for error, expected_status in (
        (CsvImportError("synthetic invalid CSV"), 422),
        (CsvImportConflictError("synthetic ambiguous account"), 409),
    ):
        coordinator = RaisingCsvCoordinator(preview_error=error)
        with TestClient(
            create_app(
                database_path=path,
                csv_import_coordinator=coordinator,  # type: ignore[arg-type]
            )
        ) as client:
            token = client.get("/dashboard").cookies["pfa_csrf"]
            response = client.post(
                "/imports/csv/preview",
                data={"csrf_token": token, "account_id": "auto"},
                files={"upload": ("synthetic.csv", b"x", "text/csv")},
                headers=SAME_ORIGIN,
            )
            assert response.status_code == expected_status

    coordinator = RaisingCsvCoordinator(
        commit_error=CsvImportConflictError("synthetic stale preview")
    )
    with TestClient(
        create_app(
            database_path=path,
            csv_import_coordinator=coordinator,  # type: ignore[arg-type]
        )
    ) as client:
        token = client.get("/dashboard").cookies["pfa_csrf"]
        conflict = client.post(
            "/imports/csv/commit",
            data={"csrf_token": token, "preview_id": "p" * 43},
            headers=SAME_ORIGIN,
        )
        assert conflict.status_code == 409
        refreshed = client.get("/dashboard?csv_imported=2&csv_refreshed=true")
        deferred = client.get("/dashboard?csv_imported=1&csv_refreshed=false")
        invalid = client.get("/dashboard?csv_imported=invalid")
        assert "The dashboard is refreshed" in refreshed.text
        assert "refresh will retry" in deferred.text
        assert "Imported invalid" not in invalid.text


def test_csv_upload_rejects_an_oversized_filename(tmp_path: Path) -> None:
    coordinator = StubCsvCoordinator()
    with TestClient(
        create_app(
            database_path=tmp_path / "state.sqlite3",
            csv_import_coordinator=coordinator,  # type: ignore[arg-type]
        )
    ) as client:
        token = client.get("/dashboard").cookies["pfa_csrf"]
        response = client.post(
            "/imports/csv/preview",
            data={"csrf_token": token, "account_id": "auto"},
            files={"upload": (f"{'x' * 252}.csv", b"x", "text/csv")},
            headers=SAME_ORIGIN,
        )
        assert response.status_code == 422
    assert not coordinator.preview_calls


def test_settings_exact_preview_apply_and_rollback(tmp_path: Path) -> None:
    client = TestClient(create_app(database_path=tmp_path / "state.sqlite3"))
    page = client.get("/dashboard")
    token = page.cookies["pfa_csrf"]
    settings = {
        "csrf_token": token,
        "timezone": "Europe/Amsterdam",
        "evaluation_hour": "17",
        "heads_up_spent_ratio": "0.70",
        "heads_up_pace_ratio": "1.20",
        "heads_up_cooldown_hours": "168",
        "warning_cooldown_hours": "72",
        "critical_cooldown_hours": "24",
    }

    preview = client.post("/settings/preview", data=settings, headers=SAME_ORIGIN)
    assert preview.status_code == 200
    assert "Confirm settings change" in preview.text
    token_marker = 'name="preview_token" value="'
    preview_token = preview.text.split(token_marker, 1)[1].split('"', 1)[0]
    applied = client.post(
        "/settings/apply",
        data={**settings, "preview_token": preview_token},
        headers=SAME_ORIGIN,
    )
    assert applied.status_code == 200
    dashboard = client.get("/v1/dashboard").json()
    assert dashboard["settings"]["evaluation_hour"] == 17
    assert dashboard["settings_revisions"][0]["actor"] == "local-owner"
    revision = dashboard["settings_revisions"][0]["revision_id"]
    rolled_back = client.post(
        f"/settings/{revision}/rollback",
        data={"csrf_token": token},
        headers=SAME_ORIGIN,
    )
    assert rolled_back.status_code == 200
    assert client.get("/v1/dashboard").json()["settings"]["evaluation_hour"] == 18


def test_host_header_is_rejected() -> None:
    response = TestClient(create_app()).get(
        "/dashboard", headers={"host": "attacker.invalid"}
    )

    assert response.status_code == 400


def test_dashboard_uses_latest_budget_evidence_for_safe_to_spend(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    policy = AlertPolicy()
    window = BudgetWindow(
        category="Synthetic dining",
        budget=Money(amount="120", currency=EUR),
        spent=Money(amount="70", currency=EUR),
        available_cash=Money(amount="1000", currency=EUR),
        upcoming_committed=Money(amount="500", currency=EUR),
        planned_savings=Money(amount="200", currency=EUR),
        minimum_buffer=Money(amount="100", currency=EUR),
        elapsed_days=3,
        total_days=7,
    )
    state.process_alert(
        budget_alert_candidate(window, evaluated_at=NOW, policy=policy), policy=policy
    )
    state.close()

    with TestClient(create_app(database_path=path)) as client:
        report = client.get("/v1/dashboard").json()["weekly"]

    assert report["safe_to_spend"][0]["amount"] == "200"
    assert report["active_alerts"] == [{"severity": "warning", "count": 1}]


def test_review_approve_reject_and_correct_lifecycle(tmp_path: Path) -> None:
    path = tmp_path / "state.sqlite3"
    seeded = SQLiteDerivedState(path)
    seeded.migrate_up()
    items = tuple(
        proposal(name) for name in ("Synthetic A", "Synthetic B", "Synthetic C")
    )
    for item in items:
        seeded.record_proposal(item)
    seeded.close()

    with TestClient(create_app(database_path=path)) as client:
        token = client.get("/dashboard").cookies["pfa_csrf"]
        approved = client.post(
            f"/review/{items[0].proposal_id}/approve",
            data={"csrf_token": token},
            headers=SAME_ORIGIN,
        )
        rejected = client.post(
            f"/review/{items[1].proposal_id}/reject",
            data={"csrf_token": token},
            headers=SAME_ORIGIN,
        )
        corrected = client.post(
            f"/review/{items[2].proposal_id}/correct",
            data={"csrf_token": token, "category": "Transport"},
            headers=SAME_ORIGIN,
        )
        assert (
            approved.status_code == rejected.status_code == corrected.status_code == 200
        )

    checked = SQLiteDerivedState(path)
    checked.migrate_up()
    statuses = {item.proposal_id: item.status for item in checked.list_proposals()}
    assert statuses == {
        items[0].proposal_id: ProposalStatus.APPROVED,
        items[1].proposal_id: ProposalStatus.REJECTED,
        items[2].proposal_id: ProposalStatus.CORRECTED,
    }
    assert checked.list_corrections()[0].approved_category == "Transport"
    checked.close()


def test_reviewed_categories_are_written_to_firefly_before_local_approval(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    seeded = SQLiteDerivedState(path)
    seeded.migrate_up()
    item = proposal("journal-1")
    seeded.record_proposal(item)
    seeded.close()
    writes: list[tuple[str, str | None, str | None]] = []

    async def update(
        transaction_id: str,
        expected_category: str | None,
        category: str | None,
    ) -> None:
        writes.append((transaction_id, expected_category, category))

    with TestClient(create_app(database_path=path, category_updater=update)) as client:
        dashboard = client.get("/v1/dashboard").json()
        assert dashboard["upstream_writes"] == "reviewed categories only"
        token = client.get("/dashboard").cookies["pfa_csrf"]
        response = client.post(
            f"/review/{item.proposal_id}/approve",
            data={"csrf_token": token},
            headers=SAME_ORIGIN,
        )

    assert response.status_code == 200
    assert writes == [("journal-1", None, "Groceries")]


def test_review_failures_are_bounded_and_do_not_mutate_demo() -> None:
    client = TestClient(create_app(demo_mode=True))
    token = client.get("/dashboard").cookies["pfa_csrf"]
    item_id = client.get("/v1/dashboard").json()["review_items"][0]["proposal_id"]

    read_only = client.post(
        f"/review/{item_id}/approve",
        data={"csrf_token": token},
        headers=SAME_ORIGIN,
        follow_redirects=False,
    )
    wrong_type = client.post(
        "/settings/preview",
        content=b"csrf_token=x",
        headers={"content-type": "text/plain"},
    )
    oversized = client.post(
        "/settings/preview",
        content=b"x" * 8193,
        headers={"content-type": "application/x-www-form-urlencoded"},
    )

    assert read_only.status_code == 409
    assert wrong_type.status_code == 415
    assert oversized.status_code == 413
