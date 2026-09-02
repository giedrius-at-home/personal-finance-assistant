"""Reversible SQLite prototype for idempotent assistant-derived state."""

from __future__ import annotations

import hashlib
import json
import sqlite3
import threading
from collections.abc import Iterable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Literal
from uuid import UUID

from pydantic import TypeAdapter

from finance_assistant.domain.alerting import (
    AlertCandidate,
    AlertDecision,
    AlertDisposition,
    AlertPolicy,
)
from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalSavingsGoal,
    CanonicalTransaction,
    SourceReference,
)
from finance_assistant.domain.models import AlertSeverity

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

MIGRATION_2_UP = """
CREATE TABLE canonical_goals (
    goal_id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(source_system, source_entity, external_id)
);
CREATE TABLE goal_source_versions (
    source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_revision TEXT,
    source_updated_at TEXT,
    observed_at TEXT NOT NULL,
    goal_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(source_system, source_entity, external_id, content_hash)
);
INSERT INTO schema_migrations(version, applied_at)
VALUES (2, CURRENT_TIMESTAMP);
"""

MIGRATION_2_DOWN = """
DROP TABLE goal_source_versions;
DROP TABLE canonical_goals;
DELETE FROM schema_migrations WHERE version = 2;
"""

MIGRATION_3_UP = """
CREATE TABLE alert_lifecycle (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    severity TEXT NOT NULL,
    last_fingerprint TEXT NOT NULL,
    cooldown_until TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(subject_kind, subject_id)
);
CREATE TABLE alert_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    severity TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    reasons_json TEXT NOT NULL,
    disposition TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);
CREATE TABLE notification_attempts (
    attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    outcome TEXT NOT NULL,
    attempted_at TEXT NOT NULL
);
CREATE TABLE evaluation_schedule (
    job_name TEXT PRIMARY KEY,
    last_success_at TEXT,
    lease_until TEXT
);
INSERT INTO schema_migrations(version, applied_at)
VALUES (3, CURRENT_TIMESTAMP);
"""

MIGRATION_3_DOWN = """
DROP TABLE evaluation_schedule;
DROP TABLE notification_attempts;
DROP TABLE alert_events;
DROP TABLE alert_lifecycle;
DELETE FROM schema_migrations WHERE version = 3;
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
            version = self._connection.execute(
                "SELECT MAX(version) FROM schema_migrations"
            ).fetchone()[0]
            if version < 2:
                self._connection.executescript(MIGRATION_2_UP)
            if version < 3:
                self._connection.executescript(MIGRATION_3_UP)

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
                          OR EXISTS(SELECT 1 FROM webhook_deliveries)
                          OR EXISTS(SELECT 1 FROM canonical_goals)
                          OR EXISTS(SELECT 1 FROM goal_source_versions)
                          OR EXISTS(SELECT 1 FROM alert_events)
                          OR EXISTS(SELECT 1 FROM notification_attempts)"""
            ).fetchone()[0]
            if populated:
                raise PersistenceConflictError(
                    "refusing to migrate down populated state"
                )
            self._connection.executescript(MIGRATION_3_DOWN)
            self._connection.executescript(MIGRATION_2_DOWN)
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

    def upsert_goal(self, goal: CanonicalSavingsGoal, *, observed_at: datetime) -> bool:
        """Persist one current goal and immutable piggy-bank source versions."""

        if observed_at.tzinfo is None:
            raise ValueError("observed_at must be timezone-aware")
        payload = json.dumps(
            goal.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        )
        content_hash = hashlib.sha256(payload.encode()).hexdigest()
        identity = _identity(goal.source)
        timestamp = observed_at.astimezone(UTC).isoformat()
        with self._lock, self._connection:
            existing = self._connection.execute(
                """SELECT goal_id, content_hash FROM canonical_goals
                   WHERE source_system = ? AND source_entity = ? AND external_id = ?""",
                identity,
            ).fetchone()
            if existing is not None and existing["goal_id"] != goal.goal.goal_id:
                raise PersistenceConflictError(
                    "piggy-bank source already has a different canonical goal"
                )
            self._connection.execute(
                """INSERT OR IGNORE INTO goal_source_versions(
                       source_system, source_entity, external_id, content_hash,
                       source_revision, source_updated_at, observed_at, goal_id,
                       payload_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    *identity,
                    content_hash,
                    goal.source.source_revision,
                    (
                        goal.source.source_updated_at.isoformat()
                        if goal.source.source_updated_at
                        else None
                    ),
                    timestamp,
                    goal.goal.goal_id,
                    payload,
                ),
            )
            if existing is not None and existing["content_hash"] == content_hash:
                return False
            self._connection.execute(
                """INSERT INTO canonical_goals(
                       goal_id, source_system, source_entity, external_id,
                       content_hash, payload_json, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(goal_id) DO UPDATE SET
                       source_system = excluded.source_system,
                       source_entity = excluded.source_entity,
                       external_id = excluded.external_id,
                       content_hash = excluded.content_hash,
                       payload_json = excluded.payload_json,
                       updated_at = excluded.updated_at""",
                (goal.goal.goal_id, *identity, content_hash, payload, timestamp),
            )
        return True

    def list_goals(self) -> tuple[CanonicalSavingsGoal, ...]:
        rows = self._connection.execute(
            "SELECT payload_json FROM canonical_goals ORDER BY goal_id"
        ).fetchall()
        adapter = TypeAdapter(CanonicalSavingsGoal)
        return tuple(adapter.validate_json(row["payload_json"]) for row in rows)

    def goal_version_count(self, source: SourceReference) -> int:
        row = self._connection.execute(
            """SELECT COUNT(*) AS count FROM goal_source_versions
               WHERE source_system = ? AND source_entity = ? AND external_id = ?""",
            _identity(source),
        ).fetchone()
        return int(row["count"])

    def process_alert(
        self, candidate: AlertCandidate, *, policy: AlertPolicy
    ) -> AlertDecision:
        """Apply deduplication, cooldown, resolution, and escalation atomically."""

        now = candidate.evaluated_at.astimezone(UTC)
        key = (candidate.subject_kind.value, candidate.subject_id)
        with self._lock, self._connection:
            current = self._connection.execute(
                """SELECT active, severity, last_fingerprint, cooldown_until
                   FROM alert_lifecycle WHERE subject_kind = ? AND subject_id = ?""",
                key,
            ).fetchone()
            if (
                current is not None
                and current["last_fingerprint"] == candidate.fingerprint
            ):
                disposition = AlertDisposition.DUPLICATE
                notify = False
            elif candidate.severity is AlertSeverity.NONE:
                if current is not None and current["active"]:
                    disposition = AlertDisposition.RESOLVED
                else:
                    disposition = AlertDisposition.INACTIVE
                notify = False
            elif current is None or not current["active"]:
                disposition = AlertDisposition.DISPATCH
                notify = True
            elif _severity_rank(candidate.severity) > _severity_rank(
                AlertSeverity(current["severity"])
            ):
                disposition = AlertDisposition.ESCALATED
                notify = True
            else:
                cooldown_until = (
                    datetime.fromisoformat(current["cooldown_until"])
                    if current["cooldown_until"]
                    else now
                )
                notify = now >= cooldown_until
                disposition = (
                    AlertDisposition.DISPATCH if notify else AlertDisposition.SUPPRESSED
                )
            active = candidate.severity is not AlertSeverity.NONE
            stored_fingerprint = (
                current["last_fingerprint"]
                if disposition is AlertDisposition.SUPPRESSED and current is not None
                else candidate.fingerprint
            )
            cooldown = (
                now + timedelta(seconds=policy.cooldown_seconds(candidate.severity))
                if notify
                else (
                    datetime.fromisoformat(current["cooldown_until"])
                    if current is not None and current["cooldown_until"]
                    else None
                )
            )
            self._connection.execute(
                """INSERT INTO alert_lifecycle(
                       subject_kind, subject_id, active, severity, last_fingerprint,
                       cooldown_until, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(subject_kind, subject_id) DO UPDATE SET
                       active = excluded.active,
                       severity = excluded.severity,
                       last_fingerprint = excluded.last_fingerprint,
                       cooldown_until = excluded.cooldown_until,
                       updated_at = excluded.updated_at""",
                (
                    *key,
                    int(active),
                    candidate.severity.value,
                    stored_fingerprint,
                    cooldown.isoformat() if cooldown else None,
                    now.isoformat(),
                ),
            )
            self._connection.execute(
                """INSERT INTO alert_events(
                       subject_kind, subject_id, severity, input_hash, fingerprint,
                       policy_version, evidence_json, reasons_json, disposition,
                       occurred_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    *key,
                    candidate.severity.value,
                    candidate.input_hash,
                    candidate.fingerprint,
                    candidate.policy_version,
                    candidate.evidence_json,
                    json.dumps(candidate.reasons, separators=(",", ":")),
                    disposition.value,
                    now.isoformat(),
                ),
            )
        return AlertDecision(
            candidate=candidate, disposition=disposition, notify=notify
        )

    def record_notification(
        self,
        candidate: AlertCandidate,
        *,
        outcome: str,
        attempted_at: datetime,
    ) -> None:
        if attempted_at.tzinfo is None:
            raise ValueError("attempted_at must be timezone-aware")
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT INTO notification_attempts(
                       subject_kind, subject_id, fingerprint, outcome, attempted_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (
                    candidate.subject_kind.value,
                    candidate.subject_id,
                    candidate.fingerprint,
                    outcome,
                    attempted_at.astimezone(UTC).isoformat(),
                ),
            )
            if outcome != "delivered":
                retry_at = attempted_at.astimezone(UTC) + timedelta(hours=1)
                self._connection.execute(
                    """UPDATE alert_lifecycle SET last_fingerprint = '',
                           cooldown_until = ?, updated_at = ?
                       WHERE subject_kind = ? AND subject_id = ?
                         AND last_fingerprint = ?""",
                    (
                        retry_at.isoformat(),
                        attempted_at.astimezone(UTC).isoformat(),
                        candidate.subject_kind.value,
                        candidate.subject_id,
                        candidate.fingerprint,
                    ),
                )

    def delete_goal(self, goal_id: str) -> bool:
        """Delete a current goal projection while retaining source history."""

        with self._lock, self._connection:
            cursor = self._connection.execute(
                "DELETE FROM canonical_goals WHERE goal_id = ?", (goal_id,)
            )
            return cursor.rowcount == 1

    def purge_alert_subject(self, subject_kind: str, subject_id: str) -> None:
        """Privacy-delete lifecycle, evidence hashes, and delivery outcomes."""

        with self._lock, self._connection:
            key = (subject_kind, subject_id)
            self._connection.execute(
                "DELETE FROM notification_attempts "
                "WHERE subject_kind = ? AND subject_id = ?",
                key,
            )
            self._connection.execute(
                "DELETE FROM alert_events WHERE subject_kind = ? AND subject_id = ?",
                key,
            )
            self._connection.execute(
                "DELETE FROM alert_lifecycle WHERE subject_kind = ? AND subject_id = ?",
                key,
            )

    def claim_evaluation(
        self,
        job_name: str,
        *,
        now: datetime,
        due_after: datetime,
        lease_seconds: int = 900,
    ) -> bool:
        """Claim one due/catch-up evaluation with an expiring crash lease."""

        if now.tzinfo is None or due_after.tzinfo is None:
            raise ValueError("scheduler timestamps must be timezone-aware")
        current_time = now.astimezone(UTC)
        threshold = due_after.astimezone(UTC)
        if current_time < threshold:
            return False
        with self._lock, self._connection:
            row = self._connection.execute(
                "SELECT last_success_at, lease_until FROM evaluation_schedule "
                "WHERE job_name = ?",
                (job_name,),
            ).fetchone()
            if row is not None:
                if (
                    row["last_success_at"]
                    and datetime.fromisoformat(row["last_success_at"]) >= threshold
                ):
                    return False
                if (
                    row["lease_until"]
                    and datetime.fromisoformat(row["lease_until"]) > current_time
                ):
                    return False
            lease_until = current_time + timedelta(seconds=lease_seconds)
            self._connection.execute(
                """INSERT INTO evaluation_schedule(job_name, lease_until)
                   VALUES (?, ?) ON CONFLICT(job_name) DO UPDATE SET
                   lease_until = excluded.lease_until""",
                (job_name, lease_until.isoformat()),
            )
            return True

    def complete_evaluation(self, job_name: str, *, completed_at: datetime) -> None:
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT INTO evaluation_schedule(
                       job_name, last_success_at, lease_until)
                   VALUES (?, ?, NULL) ON CONFLICT(job_name) DO UPDATE SET
                   last_success_at = excluded.last_success_at, lease_until = NULL""",
                (job_name, completed_at.astimezone(UTC).isoformat()),
            )

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
            self._connection.execute(
                "DELETE FROM canonical_goals WHERE source_system = ? "
                "AND source_entity = ? AND external_id = ?",
                identity,
            )
            self._connection.execute(
                "DELETE FROM goal_source_versions WHERE source_system = ? "
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


def _severity_rank(severity: AlertSeverity) -> int:
    return {
        AlertSeverity.NONE: 0,
        AlertSeverity.HEADS_UP: 1,
        AlertSeverity.WARNING: 2,
        AlertSeverity.CRITICAL: 3,
    }[severity]
