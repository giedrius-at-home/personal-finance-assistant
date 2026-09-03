"""Disposable SQLite cache for public, citation-backed research results."""

from __future__ import annotations

import sqlite3
import threading
from datetime import UTC, datetime
from pathlib import Path

from pydantic import TypeAdapter

from finance_assistant.domain.research import ResearchResult


class ResearchCache:
    """Store only sanitized public queries and bounded public documents."""

    def __init__(self, path: Path | str) -> None:
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._connection = sqlite3.connect(self.path, check_same_thread=False)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute(
            """CREATE TABLE IF NOT EXISTS research_cache (
                   query_key TEXT PRIMARY KEY,
                   purpose TEXT NOT NULL,
                   result_json TEXT NOT NULL,
                   retrieved_at TEXT NOT NULL,
                   expires_at TEXT NOT NULL
               )"""
        )
        self._connection.commit()
        self.path.chmod(0o600)

    def close(self) -> None:
        self._connection.close()

    def get(self, query_key: str) -> tuple[ResearchResult, datetime] | None:
        row = self._connection.execute(
            "SELECT result_json, expires_at FROM research_cache WHERE query_key = ?",
            (query_key,),
        ).fetchone()
        if row is None:
            return None
        return (
            TypeAdapter(ResearchResult).validate_json(row["result_json"]),
            datetime.fromisoformat(row["expires_at"]),
        )

    def put(self, result: ResearchResult) -> None:
        if not result.documents:
            raise ValueError("only cited public research can be cached")
        expires_at = min(item.citation.expires_at for item in result.documents)
        retrieved_at = min(item.citation.retrieved_at for item in result.documents)
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT INTO research_cache(
                       query_key, purpose, result_json, retrieved_at, expires_at
                   ) VALUES (?, ?, ?, ?, ?)
                   ON CONFLICT(query_key) DO UPDATE SET
                       purpose = excluded.purpose,
                       result_json = excluded.result_json,
                       retrieved_at = excluded.retrieved_at,
                       expires_at = excluded.expires_at""",
                (
                    result.query_key,
                    result.purpose.value,
                    result.model_dump_json(),
                    retrieved_at.astimezone(UTC).isoformat(),
                    expires_at.astimezone(UTC).isoformat(),
                ),
            )

    def purge(self, *, query_key: str | None = None) -> int:
        with self._lock, self._connection:
            if query_key is None:
                cursor = self._connection.execute("DELETE FROM research_cache")
            else:
                cursor = self._connection.execute(
                    "DELETE FROM research_cache WHERE query_key = ?", (query_key,)
                )
            return cursor.rowcount
