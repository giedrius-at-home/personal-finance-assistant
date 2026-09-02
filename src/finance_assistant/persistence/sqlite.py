"""Reversible SQLite prototype for idempotent assistant-derived state."""

from __future__ import annotations

import hashlib
import json
import sqlite3
import threading
from collections.abc import Iterable
from datetime import UTC, datetime
from pathlib import Path
from typing import Literal
from uuid import UUID

from pydantic import TypeAdapter

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceReference,
)

CanonicalRecord = CanonicalAccount | CanonicalTransaction
RecordKind = Literal["account", "transaction"]


class PersistenceConflictError(RuntimeError):
    """A source identity was assigned conflicting canonical ownership."""


MIGRATION_1_UP = """
CREATE TABLE source_versions (
    source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_revision TEXT,
    source_updated_at TEXT,
    observed_at TEXT NOT NULL,
    record_kind TEXT NOT NULL CHECK(record_kind IN ('account', 'transaction')),
    canonical_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (source_system, source_entity, external_id, content_hash)
);
CREATE TABLE canonical_records (
    record_kind TEXT NOT NULL CHECK(record_kind IN ('account', 'transaction')),
    canonical_id TEXT NOT NULL,
    source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (record_kind, canonical_id),
    UNIQUE (source_system, source_entity, external_id)
);
CREATE TABLE webhook_deliveries (
    delivery_id TEXT PRIMARY KEY,
    accepted_at INTEGER NOT NULL,
    CHECK(length(delivery_id) = 36)
);
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
INSERT INTO schema_migrations(version, applied_at)
VALUES (1, CURRENT_TIMESTAMP);
"""

MIGRATION_1_DOWN = """
DROP TABLE webhook_deliveries;
DROP TABLE canonical_records;
DROP TABLE source_versions;
DROP TABLE schema_migrations;
"""


def _serialize(record: CanonicalRecord) -> str:
    return json.dumps(
        record.model_dump(mode="json"),
        sort_keys=True,
        separators=(",", ":"),
    )


def _identity(source: SourceReference) -> tuple[str, str, str]:
    return (source.system.value, source.entity.value, source.external_id)


def _record_parts(
    record: CanonicalRecord,
) -> tuple[RecordKind, str, SourceReference]:
    if isinstance(record, CanonicalAccount):
        return "account", record.account_id, record.canonical_owner
    return "transaction", record.transaction_id, record.source


class SQLiteDerivedState:
    """Store current projections plus immutable source versions transactionally."""

    def __init__(self, path: Path | str) -> None:
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._connection = sqlite3.connect(self.path, check_same_thread=False)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA foreign_keys = ON")
        self._connection.execute("PRAGMA journal_mode = WAL")

    def close(self) -> None:
        self._connection.close()

    def migrate_up(self) -> None:
        with self._lock:
            existing = self._connection.execute(
                "SELECT name FROM sqlite_master WHERE name = 'schema_migrations'"
            ).fetchone()
            if existing is None:
                self._connection.executescript(MIGRATION_1_UP)

    def migrate_down(self) -> None:
        """Reverse the prototype schema only when no financial state exists."""

        with self._lock:
            if (
                self._connection.execute(
                    "SELECT name FROM sqlite_master WHERE name = 'schema_migrations'"
                ).fetchone()
                is None
            ):
                return
            populated = self._connection.execute(
                """SELECT EXISTS(SELECT 1 FROM source_versions)
                          OR EXISTS(SELECT 1 FROM canonical_records)
                          OR EXISTS(SELECT 1 FROM webhook_deliveries)"""
            ).fetchone()[0]
            if populated:
                raise PersistenceConflictError(
                    "refusing to migrate down populated state"
                )
            self._connection.executescript(MIGRATION_1_DOWN)

    def upsert(self, record: CanonicalRecord, *, observed_at: datetime) -> bool:
        """Insert/update one projection; return false for an exact replay."""

        if observed_at.tzinfo is None:
            raise ValueError("observed_at must be timezone-aware")
        kind, canonical_id, source = _record_parts(record)
        payload = _serialize(record)
        content_hash = hashlib.sha256(payload.encode()).hexdigest()
        source_identity = _identity(source)
        lineage = (
            (record.canonical_owner, *record.linked_sources)
            if isinstance(record, CanonicalAccount)
            else (source,)
        )
        with self._lock, self._connection:
            existing = self._connection.execute(
                """SELECT canonical_id, content_hash FROM canonical_records
                   WHERE source_system = ? AND source_entity = ? AND external_id = ?""",
                source_identity,
            ).fetchone()
            if existing is not None and existing["canonical_id"] != canonical_id:
                raise PersistenceConflictError(
                    "source identity already has a different canonical owner"
                )
            for reference in lineage:
                self._connection.execute(
                    """INSERT OR IGNORE INTO source_versions(
                           source_system, source_entity, external_id, content_hash,
                           source_revision, source_updated_at, observed_at,
                           record_kind, canonical_id, payload_json
                       ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (
                        *_identity(reference),
                        content_hash,
                        reference.source_revision,
                        (
                            reference.source_updated_at.isoformat()
                            if reference.source_updated_at
                            else None
                        ),
                        observed_at.astimezone(UTC).isoformat(),
                        kind,
                        canonical_id,
                        payload,
                    ),
                )
            if existing is not None and existing["content_hash"] == content_hash:
                return False
            self._connection.execute(
                """INSERT INTO canonical_records(
                       record_kind, canonical_id, source_system, source_entity,
                       external_id, content_hash, payload_json, updated_at
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(record_kind, canonical_id) DO UPDATE SET
                       source_system = excluded.source_system,
                       source_entity = excluded.source_entity,
                       external_id = excluded.external_id,
                       content_hash = excluded.content_hash,
                       payload_json = excluded.payload_json,
                       updated_at = excluded.updated_at""",
                (
                    kind,
                    canonical_id,
                    *source_identity,
                    content_hash,
                    payload,
                    observed_at.astimezone(UTC).isoformat(),
                ),
            )
        return True

    def upsert_many(
        self, records: Iterable[CanonicalRecord], *, observed_at: datetime
    ) -> int:
        return sum(self.upsert(record, observed_at=observed_at) for record in records)

    def list_accounts(self) -> tuple[CanonicalAccount, ...]:
        rows = self._connection.execute(
            "SELECT payload_json FROM canonical_records "
            "WHERE record_kind = 'account' ORDER BY canonical_id"
        ).fetchall()
        adapter = TypeAdapter(CanonicalAccount)
        return tuple(adapter.validate_json(row["payload_json"]) for row in rows)

    def list_transactions(self) -> tuple[CanonicalTransaction, ...]:
        rows = self._connection.execute(
            "SELECT payload_json FROM canonical_records "
            "WHERE record_kind = 'transaction' ORDER BY canonical_id"
        ).fetchall()
        adapter = TypeAdapter(CanonicalTransaction)
        return tuple(adapter.validate_json(row["payload_json"]) for row in rows)

    def version_count(self, source: SourceReference) -> int:
        row = self._connection.execute(
            """SELECT COUNT(*) AS count FROM source_versions
               WHERE source_system = ? AND source_entity = ? AND external_id = ?""",
            _identity(source),
        ).fetchone()
        return int(row["count"])

    def claim(self, delivery_id: UUID, *, now: int) -> bool:
        """Atomically retain a webhook UUID across process restarts."""

        with self._lock, self._connection:
            cursor = self._connection.execute(
                "INSERT OR IGNORE INTO webhook_deliveries(delivery_id, accepted_at) "
                "VALUES (?, ?)",
                (str(delivery_id), now),
            )
            return cursor.rowcount == 1

    def delete_current(self, kind: RecordKind, canonical_id: str) -> bool:
        """Delete a derived projection while retaining immutable audit lineage."""

        with self._lock, self._connection:
            cursor = self._connection.execute(
                "DELETE FROM canonical_records "
                "WHERE record_kind = ? AND canonical_id = ?",
                (kind, canonical_id),
            )
            return cursor.rowcount == 1

    def purge_source(self, source: SourceReference) -> None:
        """Privacy-delete current and historical state for one source identity."""

        identity = _identity(source)
        with self._lock, self._connection:
            self._connection.execute(
                "DELETE FROM canonical_records WHERE source_system = ? "
                "AND source_entity = ? AND external_id = ?",
                identity,
            )
            self._connection.execute(
                "DELETE FROM source_versions WHERE source_system = ? "
                "AND source_entity = ? AND external_id = ?",
                identity,
            )

    def backup_to(self, destination: Path) -> None:
        """Create a consistent SQLite backup without exposing record contents."""

        if destination.exists():
            raise FileExistsError(destination)
        target = sqlite3.connect(destination)
        try:
            with self._lock:
                self._connection.backup(target)
        finally:
            target.close()
