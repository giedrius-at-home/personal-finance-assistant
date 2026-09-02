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

from finance_assistant.categorisation_service import (
    CategoryProposal,
    ProposalStatus,
    source_identity,
)
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
from finance_assistant.domain.categorisation import CalibrationExample
from finance_assistant.domain.models import AlertSeverity, BudgetEvaluation, Money
from finance_assistant.domain.settings import OwnerSettings, SettingsPreview

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

MIGRATION_4_UP = """
CREATE TABLE category_proposals (
    proposal_id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_entity TEXT NOT NULL,
    external_id TEXT NOT NULL,
    feature_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'approved', 'corrected', 'rejected', 'reversed'
    )),
    batch_id TEXT,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE category_corrections (
    correction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_identity TEXT NOT NULL,
    feature_hash TEXT NOT NULL,
    normalized_merchant TEXT NOT NULL,
    approved_category TEXT NOT NULL,
    approved_at TEXT NOT NULL,
    UNIQUE(source_identity, feature_hash, approved_category)
);
CREATE TABLE category_batches (
    batch_id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK(status IN ('approved', 'reversed')),
    created_at TEXT NOT NULL
);
INSERT INTO schema_migrations(version, applied_at)
VALUES (4, CURRENT_TIMESTAMP);
"""

MIGRATION_4_DOWN = """
DROP TABLE category_batches;
DROP TABLE category_corrections;
DROP TABLE category_proposals;
DELETE FROM schema_migrations WHERE version = 4;
"""

MIGRATION_5_UP = """
CREATE TABLE owner_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE settings_history (
    revision_id INTEGER PRIMARY KEY AUTOINCREMENT,
    before_hash TEXT NOT NULL,
    after_hash TEXT NOT NULL,
    before_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    preview_token TEXT NOT NULL UNIQUE,
    actor TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    rolled_back_at TEXT,
    rolled_back_by TEXT
);
INSERT INTO schema_migrations(version, applied_at)
VALUES (5, CURRENT_TIMESTAMP);
"""

MIGRATION_5_DOWN = """
DROP TABLE settings_history;
DROP TABLE owner_settings;
DELETE FROM schema_migrations WHERE version = 5;
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


def _proposal_token_payload(proposal: CategoryProposal) -> str:
    """Canonical serialization of immutable proposal content for batch binding."""

    return json.dumps(
        proposal.model_dump(mode="json", exclude={"status", "created_at"}),
        sort_keys=True,
        separators=(",", ":"),
    )


def _batch_token(proposals: tuple[CategoryProposal, ...]) -> str:
    payloads = sorted(_proposal_token_payload(proposal) for proposal in proposals)
    return hashlib.sha256("\n".join(payloads).encode()).hexdigest()


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
            if version < 4:
                self._connection.executescript(MIGRATION_4_UP)
            if version < 5:
                self._connection.executescript(MIGRATION_5_UP)

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
                          OR EXISTS(SELECT 1 FROM notification_attempts)
                          OR EXISTS(SELECT 1 FROM category_proposals)
                          OR EXISTS(SELECT 1 FROM category_corrections)
                          OR EXISTS(SELECT 1 FROM settings_history)"""
            ).fetchone()[0]
            if populated:
                raise PersistenceConflictError(
                    "refusing to migrate down populated state"
                )
            self._connection.executescript(MIGRATION_5_DOWN)
            self._connection.executescript(MIGRATION_4_DOWN)
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

    def record_proposal(self, proposal: CategoryProposal) -> None:
        """Idempotently persist a reviewable categorisation proposal."""

        if proposal.created_at.tzinfo is None:
            raise ValueError("proposal.created_at must be timezone-aware")
        payload = json.dumps(
            proposal.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        )
        timestamp = proposal.created_at.astimezone(UTC).isoformat()
        source = proposal.source
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT OR IGNORE INTO category_proposals(
                       proposal_id, source_system, source_entity, external_id,
                       feature_hash, status, batch_id, payload_json, created_at,
                       updated_at) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)""",
                (
                    proposal.proposal_id,
                    source.system.value,
                    source.entity.value,
                    source.external_id,
                    proposal.feature_hash,
                    proposal.status.value,
                    payload,
                    timestamp,
                    timestamp,
                ),
            )

    def list_pending(self) -> tuple[CategoryProposal, ...]:
        rows = self._connection.execute(
            "SELECT payload_json FROM category_proposals "
            "WHERE status = 'pending' ORDER BY created_at"
        ).fetchall()
        adapter = TypeAdapter(CategoryProposal)
        return tuple(adapter.validate_json(row["payload_json"]) for row in rows)

    def list_proposals(self, *, limit: int = 100) -> tuple[CategoryProposal, ...]:
        """Return bounded newest review items with their current lifecycle state."""

        if not 1 <= limit <= 500:
            raise ValueError("proposal limit must be between 1 and 500")
        rows = self._connection.execute(
            "SELECT status, payload_json FROM category_proposals "
            "ORDER BY created_at DESC LIMIT ?",
            (limit,),
        ).fetchall()
        adapter = TypeAdapter(CategoryProposal)
        proposals: list[CategoryProposal] = []
        for row in rows:
            payload = json.loads(row["payload_json"])
            payload["status"] = row["status"]
            proposals.append(adapter.validate_python(payload))
        return tuple(proposals)

    def get_pending(self, proposal_id: str) -> CategoryProposal | None:
        row = self._connection.execute(
            "SELECT payload_json FROM category_proposals "
            "WHERE proposal_id = ? AND status = 'pending'",
            (proposal_id,),
        ).fetchone()
        if row is None:
            return None
        return TypeAdapter(CategoryProposal).validate_json(row["payload_json"])

    def preview_batch(self, proposal_ids: tuple[str, ...]) -> str:
        return _batch_token(self._require_pending(proposal_ids))

    def validate_category_batch(
        self, proposal_ids: tuple[str, ...], *, preview_token: str
    ) -> tuple[CategoryProposal, ...]:
        proposals = self._require_pending(proposal_ids)
        if _batch_token(proposals) != preview_token:
            raise PersistenceConflictError(
                "preview token does not match the current proposals"
            )
        return proposals

    def approve_batch(
        self, proposal_ids: tuple[str, ...], *, preview_token: str
    ) -> tuple[CategoryProposal, ...]:
        with self._lock, self._connection:
            self.validate_category_batch(proposal_ids, preview_token=preview_token)
            batch_id = preview_token
            now = datetime.now(UTC).isoformat()
            placeholders = ",".join("?" for _ in proposal_ids)
            self._connection.execute(
                f"UPDATE category_proposals SET status = 'approved', "  # noqa: S608
                f"batch_id = ?, updated_at = ? WHERE proposal_id IN ({placeholders}) "  # nosec B608
                f"AND status = 'pending'",
                (batch_id, now, *proposal_ids),
            )
            self._connection.execute(
                """INSERT OR IGNORE INTO category_batches(batch_id, status, created_at)
                   VALUES (?, 'approved', ?)""",
                (batch_id, now),
            )
        return self._load_proposals(proposal_ids)

    def record_correction(
        self, example: CalibrationExample, *, approved_at: datetime | None = None
    ) -> None:
        timestamp = approved_at or datetime.now(UTC)
        if timestamp.tzinfo is None:
            raise ValueError("approved_at must be timezone-aware")
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT OR IGNORE INTO category_corrections(
                       source_identity, feature_hash, normalized_merchant,
                       approved_category, approved_at) VALUES (?, ?, ?, ?, ?)""",
                (
                    example.source_identity,
                    example.feature_hash,
                    example.normalized_merchant,
                    example.approved_category,
                    timestamp.astimezone(UTC).isoformat(),
                ),
            )

    def list_corrections(self) -> tuple[CalibrationExample, ...]:
        rows = self._connection.execute(
            """SELECT source_identity, feature_hash, normalized_merchant,
                      approved_category FROM category_corrections
               ORDER BY correction_id"""
        ).fetchall()
        return tuple(
            CalibrationExample(
                source_identity=row["source_identity"],
                feature_hash=row["feature_hash"],
                normalized_merchant=row["normalized_merchant"],
                approved_category=row["approved_category"],
            )
            for row in rows
        )

    def resolve_proposal(self, proposal_id: str, *, status: ProposalStatus) -> bool:
        with self._lock, self._connection:
            cursor = self._connection.execute(
                "UPDATE category_proposals SET status = ?, updated_at = ? "
                "WHERE proposal_id = ? AND status = 'pending'",
                (status.value, datetime.now(UTC).isoformat(), proposal_id),
            )
            return cursor.rowcount == 1

    def reverse_batch(self, batch_id: str) -> int:
        with self._lock, self._connection:
            now = datetime.now(UTC).isoformat()
            self._connection.execute(
                "UPDATE category_batches SET status = 'reversed' WHERE batch_id = ?",
                (batch_id,),
            )
            cursor = self._connection.execute(
                "UPDATE category_proposals SET status = 'reversed', updated_at = ? "
                "WHERE batch_id = ? AND status = 'approved'",
                (now, batch_id),
            )
            return cursor.rowcount

    def category_batch_is_approved(self, batch_id: str) -> bool:
        row = self._connection.execute(
            "SELECT status FROM category_batches WHERE batch_id = ?", (batch_id,)
        ).fetchone()
        return row is not None and row["status"] == "approved"

    def list_active_alerts(
        self, *, limit: int = 100
    ) -> tuple[tuple[str, str, AlertSeverity, str], ...]:
        """Return bounded content-free alert summaries for local review."""

        if not 1 <= limit <= 500:
            raise ValueError("alert limit must be between 1 and 500")
        rows = self._connection.execute(
            "SELECT subject_kind, subject_id, severity, updated_at "
            "FROM alert_lifecycle WHERE active = 1 "
            "ORDER BY updated_at DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return tuple(
            (
                str(row["subject_kind"]),
                str(row["subject_id"]),
                AlertSeverity(row["severity"]),
                str(row["updated_at"]),
            )
            for row in rows
        )

    def list_safe_to_spend(self) -> tuple[Money, ...]:
        """Return latest exact budget evidence for conservative currency totals."""

        rows = self._connection.execute(
            """SELECT evidence_json FROM alert_events
               WHERE event_id IN (
                   SELECT MAX(event_id) FROM alert_events
                   WHERE subject_kind = 'budget' GROUP BY subject_id
               ) ORDER BY subject_id"""
        ).fetchall()
        adapter = TypeAdapter(BudgetEvaluation)
        return tuple(
            Money(
                amount=evaluation.safe_to_spend,
                currency=evaluation.currency,
            )
            for row in rows
            for evaluation in (adapter.validate_json(row["evidence_json"]),)
        )

    def get_settings(self) -> OwnerSettings:
        row = self._connection.execute(
            "SELECT payload_json FROM owner_settings WHERE singleton = 1"
        ).fetchone()
        if row is None:
            return OwnerSettings()
        return TypeAdapter(OwnerSettings).validate_json(row["payload_json"])

    def preview_settings(self, desired: OwnerSettings) -> SettingsPreview:
        before = self.get_settings()
        token = hashlib.sha256(
            f"settings-preview\0{before.content_hash}\0{desired.content_hash}".encode()
        ).hexdigest()
        return SettingsPreview(token=token, before=before, after=desired)

    def apply_settings(
        self,
        preview: SettingsPreview,
        *,
        actor: str = "local-owner",
        applied_at: datetime | None = None,
    ) -> int:
        """Atomically apply the exact preview when current settings are unchanged."""

        if not actor or len(actor) > 64 or actor != actor.strip():
            raise ValueError("actor must be a bounded non-empty identifier")
        timestamp = applied_at or datetime.now(UTC)
        if timestamp.tzinfo is None:
            raise ValueError("applied_at must be timezone-aware")
        expected = self.preview_settings(preview.after)
        if expected != preview:
            raise PersistenceConflictError("settings changed after preview")
        before_json = preview.before.model_dump_json()
        after_json = preview.after.model_dump_json()
        with self._lock, self._connection:
            current = self.get_settings()
            if current.content_hash != preview.before.content_hash:
                raise PersistenceConflictError("settings changed after preview")
            cursor = self._connection.execute(
                """INSERT INTO settings_history(
                       before_hash, after_hash, before_json, after_json,
                       preview_token, actor, applied_at, active)
                   VALUES (?, ?, ?, ?, ?, ?, ?, 1)""",
                (
                    preview.before.content_hash,
                    preview.after.content_hash,
                    before_json,
                    after_json,
                    preview.token,
                    actor,
                    timestamp.astimezone(UTC).isoformat(),
                ),
            )
            self._connection.execute(
                """INSERT INTO owner_settings(
                       singleton, content_hash, payload_json, updated_at)
                   VALUES (1, ?, ?, ?)
                   ON CONFLICT(singleton) DO UPDATE SET
                       content_hash = excluded.content_hash,
                       payload_json = excluded.payload_json,
                       updated_at = excluded.updated_at""",
                (
                    preview.after.content_hash,
                    after_json,
                    timestamp.astimezone(UTC).isoformat(),
                ),
            )
            if cursor.lastrowid is None:  # pragma: no cover - SQLite INSERT invariant
                raise RuntimeError("settings revision was not created")
            return cursor.lastrowid

    def rollback_settings(
        self,
        revision_id: int,
        *,
        actor: str = "local-owner",
        rolled_back_at: datetime | None = None,
    ) -> bool:
        if not actor or len(actor) > 64 or actor != actor.strip():
            raise ValueError("actor must be a bounded non-empty identifier")
        timestamp = rolled_back_at or datetime.now(UTC)
        if timestamp.tzinfo is None:
            raise ValueError("rolled_back_at must be timezone-aware")
        with self._lock, self._connection:
            row = self._connection.execute(
                "SELECT before_hash, after_hash, before_json, active "
                "FROM settings_history WHERE revision_id = ?",
                (revision_id,),
            ).fetchone()
            if row is None or not row["active"]:
                return False
            current = self.get_settings()
            if current.content_hash != row["after_hash"]:
                raise PersistenceConflictError(
                    "newer settings prevent rollback of this revision"
                )
            self._connection.execute(
                "UPDATE owner_settings SET content_hash = ?, payload_json = ?, "
                "updated_at = ? WHERE singleton = 1",
                (
                    row["before_hash"],
                    row["before_json"],
                    timestamp.astimezone(UTC).isoformat(),
                ),
            )
            self._connection.execute(
                "UPDATE settings_history SET active = 0, rolled_back_at = ?, "
                "rolled_back_by = ? WHERE revision_id = ?",
                (timestamp.astimezone(UTC).isoformat(), actor, revision_id),
            )
            return True

    def list_settings_revisions(
        self,
    ) -> tuple[tuple[int, str, str, bool, str | None, str | None], ...]:
        rows = self._connection.execute(
            "SELECT revision_id, actor, applied_at, active, rolled_back_at, "
            "rolled_back_by FROM settings_history "
            "ORDER BY revision_id DESC LIMIT 50"
        ).fetchall()
        return tuple(
            (
                int(row["revision_id"]),
                str(row["actor"]),
                str(row["applied_at"]),
                bool(row["active"]),
                str(row["rolled_back_at"]) if row["rolled_back_at"] else None,
                str(row["rolled_back_by"]) if row["rolled_back_by"] else None,
            )
            for row in rows
        )

    def _require_pending(
        self, proposal_ids: tuple[str, ...]
    ) -> tuple[CategoryProposal, ...]:
        if not proposal_ids or len(set(proposal_ids)) != len(proposal_ids):
            raise ValueError("proposal_ids must be non-empty and unique")
        placeholders = ",".join("?" for _ in proposal_ids)
        rows = self._connection.execute(
            f"SELECT payload_json FROM category_proposals "  # noqa: S608
            f"WHERE proposal_id IN ({placeholders}) AND status = 'pending'",  # nosec B608
            proposal_ids,
        ).fetchall()
        if len(rows) != len(proposal_ids):
            raise PersistenceConflictError("a proposal is missing or not pending")
        adapter = TypeAdapter(CategoryProposal)
        return tuple(adapter.validate_json(row["payload_json"]) for row in rows)

    def _load_proposals(
        self, proposal_ids: tuple[str, ...]
    ) -> tuple[CategoryProposal, ...]:
        placeholders = ",".join("?" for _ in proposal_ids)
        rows = self._connection.execute(
            f"SELECT status, payload_json FROM category_proposals "  # noqa: S608
            f"WHERE proposal_id IN ({placeholders})",  # nosec B608
            proposal_ids,
        ).fetchall()
        adapter = TypeAdapter(CategoryProposal)
        proposals: list[CategoryProposal] = []
        for row in rows:
            data = json.loads(row["payload_json"])
            data["status"] = row["status"]
            proposals.append(adapter.validate_python(data))
        return tuple(proposals)

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
            self._connection.execute(
                "DELETE FROM category_proposals WHERE source_system = ? "
                "AND source_entity = ? AND external_id = ?",
                identity,
            )
            self._connection.execute(
                "DELETE FROM category_corrections WHERE source_identity = ?",
                (source_identity(source),),
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
