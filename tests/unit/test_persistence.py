import sqlite3
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime
from pathlib import Path
from uuid import uuid4

import pytest

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)

NOW = datetime(2026, 1, 2, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def account(balance: str = "125.50", revision: str = "r1") -> CanonicalAccount:
    return CanonicalAccount(
        account_id="firefly:synthetic-account",
        name="Synthetic cash",
        balance=Money(amount=balance, currency=EUR),
        canonical_owner=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.ACCOUNT,
            external_id="synthetic-account",
            source_revision=revision,
            source_updated_at=NOW,
        ),
    )


def test_repeated_import_is_idempotent_and_update_preserves_lineage(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    initial = account()
    updated = account("130.75", "r2")

    assert state.upsert(initial, observed_at=NOW)
    assert not state.upsert(initial, observed_at=NOW)
    assert state.upsert(updated, observed_at=NOW)
    assert state.list_accounts() == (updated,)
    assert state.version_count(initial.canonical_owner) == 2
    state.close()


def test_source_cannot_silently_change_canonical_owner(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    initial = account()
    state.upsert(initial, observed_at=NOW)

    with pytest.raises(PersistenceConflictError, match="different canonical owner"):
        state.upsert(
            initial.model_copy(update={"account_id": "firefly:different"}),
            observed_at=NOW,
        )
    state.close()


def test_durable_delivery_claim_is_atomic_and_survives_restart(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    delivery_id = uuid4()

    with ThreadPoolExecutor(max_workers=8) as executor:
        results = list(
            executor.map(
                lambda _: state.claim(delivery_id, now=1_800_000_000), range(32)
            )
        )
    assert results.count(True) == 1
    state.close()

    reopened = SQLiteDerivedState(path)
    reopened.migrate_up()
    assert not reopened.claim(delivery_id, now=1_800_000_100)
    reopened.close()


def test_projection_delete_retains_lineage_and_privacy_purge_removes_it(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    record = account()
    state.upsert(record, observed_at=NOW)

    assert state.delete_current("account", record.account_id)
    assert state.list_accounts() == ()
    assert state.version_count(record.canonical_owner) == 1
    state.purge_source(record.canonical_owner)
    assert state.version_count(record.canonical_owner) == 0
    state.close()


def test_migration_is_reversible_only_before_state_exists(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "empty.sqlite3")
    state.migrate_up()
    state.migrate_down()
    database = sqlite3.connect(state.path)
    try:
        tables = database.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table'"
        ).fetchall()
    finally:
        database.close()
    assert tables == []
    state.migrate_up()
    state.upsert(account(), observed_at=NOW)
    with pytest.raises(PersistenceConflictError, match="populated"):
        state.migrate_down()
    state.close()


def test_backup_is_consistent_and_refuses_overwrite(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    state.upsert(account(), observed_at=NOW)
    destination = tmp_path / "backup.sqlite3"

    state.backup_to(destination)
    backup = sqlite3.connect(destination)
    try:
        assert backup.execute("SELECT COUNT(*) FROM canonical_records").fetchone() == (
            1,
        )
    finally:
        backup.close()
    with pytest.raises(FileExistsError):
        state.backup_to(destination)
    state.close()
