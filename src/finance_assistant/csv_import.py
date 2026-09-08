"""Provider-neutral, duplicate-safe CSV staging for reviewed Firefly imports."""

from __future__ import annotations

import csv
import hashlib
import io
import re
import secrets
import threading
from collections import Counter
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal, InvalidOperation
from enum import StrEnum
from pathlib import Path
from types import TracebackType
from typing import Protocol, Self

from pydantic import BaseModel, ConfigDict, Field, field_validator

from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyTransaction,
)

MAX_CSV_BYTES = 5 * 1024 * 1024
MAX_CSV_ROWS = 10_000
MAX_PREVIEW_ROWS = 100
PREVIEW_TTL = timedelta(minutes=30)


class CsvImportError(ValueError):
    """Bounded upload error that never includes row contents."""


class CsvImportConflictError(RuntimeError):
    """The staged import is missing, expired, or no longer safe to apply."""


class CsvRowStatus(StrEnum):
    NEW = "new"
    DUPLICATE = "duplicate"
    POSSIBLE_DUPLICATE = "possible_duplicate"


class ParsedCsvRow(BaseModel):
    """Normalized transaction row retained only in the in-memory preview store."""

    model_config = ConfigDict(frozen=True)

    line_number: int = Field(ge=2, le=MAX_CSV_ROWS + 1)
    booked_on: date
    amount: Decimal = Field(max_digits=38)
    currency: str = Field(min_length=3, max_length=3)
    description: str = Field(min_length=1, max_length=1024)
    external_reference: str | None = Field(default=None, max_length=512)
    counterparty_iban: str | None = Field(default=None, max_length=64)

    @field_validator("amount")
    @classmethod
    def nonzero_amount(cls, value: Decimal) -> Decimal:
        if not value.is_finite() or value == 0:
            raise ValueError("amount must be finite and non-zero")
        return value

    @field_validator("currency")
    @classmethod
    def currency_code(cls, value: str) -> str:
        normalized = value.strip().upper()
        if len(normalized) != 3 or not normalized.isascii() or not normalized.isalpha():
            raise ValueError("currency must be a three-letter code")
        return normalized


class CsvPreviewRow(BaseModel):
    """One owner-visible row in a bounded local preview."""

    model_config = ConfigDict(frozen=True)

    booked_on: date
    amount: Decimal
    currency: str
    description: str
    status: CsvRowStatus


class CsvImportPreview(BaseModel):
    """Review summary; the opaque ID is required to commit the exact snapshot."""

    model_config = ConfigDict(frozen=True)

    preview_id: str = Field(min_length=32, max_length=128)
    bank_label: str = Field(min_length=1, max_length=128)
    account_name: str = Field(min_length=1, max_length=256)
    row_count: int = Field(ge=0)
    new_count: int = Field(ge=0)
    duplicate_count: int = Field(ge=0)
    possible_duplicate_count: int = Field(ge=0)
    rejected_count: int = Field(ge=0)
    rows: tuple[CsvPreviewRow, ...] = Field(max_length=MAX_PREVIEW_ROWS)

    @property
    def committable(self) -> bool:
        return self.rejected_count == 0 and self.possible_duplicate_count == 0


@dataclass(frozen=True, slots=True)
class StagedCsvImport:
    preview: CsvImportPreview
    account_id: str
    rows: tuple[ParsedCsvRow, ...]
    statuses: tuple[CsvRowStatus, ...]
    created_at: datetime


class CsvImportReader(Protocol):
    async def list_accounts(self) -> tuple[FireflyAccount, ...]: ...

    async def list_transactions(
        self, *, start: date | None = None, end: date | None = None
    ) -> tuple[FireflyTransaction, ...]: ...


class CsvImportWriter(CsvImportReader, Protocol):
    async def __aenter__(self) -> Self: ...

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...

    async def create_csv_transaction(
        self,
        *,
        row: ParsedCsvRow,
        account: FireflyAccount,
        accounts: tuple[FireflyAccount, ...],
        external_id: str,
    ) -> None: ...


class CsvPreviewStore:
    """Short-lived, process-local storage; raw uploads are never written to disk."""

    def __init__(self) -> None:
        self._items: dict[str, StagedCsvImport] = {}
        self._lock = threading.Lock()

    def put(self, staged: StagedCsvImport) -> None:
        with self._lock:
            self._items[staged.preview.preview_id] = staged

    def pop(self, preview_id: str, *, now: datetime) -> StagedCsvImport:
        with self._lock:
            staged = self._items.pop(preview_id, None)
            self._purge(now)
        if staged is None or now - staged.created_at > PREVIEW_TTL:
            raise CsvImportConflictError("CSV preview is missing or expired")
        return staged

    def _purge(self, now: datetime) -> None:
        expired = [
            key
            for key, value in self._items.items()
            if now - value.created_at > PREVIEW_TTL
        ]
        for key in expired:
            del self._items[key]


class CsvImportCoordinator:
    """Parse, compare, stage, and explicitly commit bank CSV transactions."""

    def __init__(
        self,
        *,
        client_factory: Callable[[], CsvImportWriter],
        store: CsvPreviewStore | None = None,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        self._client_factory = client_factory
        self._store = store or CsvPreviewStore()
        self._clock = clock or (lambda: datetime.now(UTC))

    async def preview(
        self, *, content: bytes, filename: str, account_id: str
    ) -> CsvImportPreview:
        rows, rejected = parse_bank_csv(content, filename=filename)
        bank_label = detect_bank_label(filename, rows)
        async with self._client_factory() as client:
            accounts = await client.list_accounts()
            account = _one_account(accounts, account_id, bank_label=bank_label)
            _validate_account_currency(rows, account)
            existing = await client.list_transactions(
                start=min(row.booked_on for row in rows),
                end=max(row.booked_on for row in rows),
            )
        statuses = _classify_rows(
            rows,
            account=account,
            existing=existing,
            bank_label=bank_label,
        )
        preview = CsvImportPreview(
            preview_id=secrets.token_urlsafe(32),
            bank_label=bank_label,
            account_name=account.attributes.name,
            row_count=len(rows) + rejected,
            new_count=statuses.count(CsvRowStatus.NEW),
            duplicate_count=statuses.count(CsvRowStatus.DUPLICATE),
            possible_duplicate_count=statuses.count(CsvRowStatus.POSSIBLE_DUPLICATE),
            rejected_count=rejected,
            rows=tuple(
                CsvPreviewRow(
                    booked_on=row.booked_on,
                    amount=row.amount,
                    currency=row.currency,
                    description=row.description,
                    status=row_status,
                )
                for row, row_status in zip(rows, statuses, strict=True)
            )[:MAX_PREVIEW_ROWS],
        )
        self._store.put(
            StagedCsvImport(
                preview=preview,
                account_id=account.id,
                rows=rows,
                statuses=statuses,
                created_at=self._clock(),
            )
        )
        return preview

    async def commit(self, preview_id: str) -> int:
        staged = self._store.pop(preview_id, now=self._clock())
        if not staged.preview.committable:
            raise CsvImportConflictError(
                "CSV preview contains rejected or possible duplicate rows"
            )
        async with self._client_factory() as client:
            accounts = await client.list_accounts()
            account = _one_account(
                accounts, staged.account_id, bank_label=staged.preview.bank_label
            )
            existing = await client.list_transactions(
                start=min(row.booked_on for row in staged.rows),
                end=max(row.booked_on for row in staged.rows),
            )
            current_statuses = _classify_rows(
                staged.rows,
                account=account,
                existing=existing,
                bank_label=staged.preview.bank_label,
            )
            if current_statuses != staged.statuses:
                raise CsvImportConflictError(
                    "Firefly changed after the CSV preview was created"
                )
            imported = 0
            for row, status_value in zip(staged.rows, staged.statuses, strict=True):
                if status_value is not CsvRowStatus.NEW:
                    continue
                await client.create_csv_transaction(
                    row=row,
                    account=account,
                    accounts=accounts,
                    external_id=csv_external_id(
                        row, account_id=account.id, bank_label=staged.preview.bank_label
                    ),
                )
                imported += 1
        return imported


HEADER_ALIASES: dict[str, frozenset[str]] = {
    "date": frozenset(
        {
            "date",
            "booking date",
            "book date",
            "transaction date",
            "completed date",
            "datum",
            "boekdatum",
            "data",
        }
    ),
    "amount": frozenset(
        {"amount", "transaction amount", "value", "bedrag", "sum", "suma"}
    ),
    "debit": frozenset({"debit", "debit amount", "af", "expenses"}),
    "credit": frozenset({"credit", "credit amount", "bij", "income"}),
    "currency": frozenset({"currency", "currency code", "valuta", "valiuta", "ccy"}),
    "description": frozenset(
        {
            "description",
            "details",
            "merchant",
            "recipient",
            "beneficiary",
            "omschrijving",
            "naam omschrijving",
            "memo",
        }
    ),
    "reference": frozenset(
        {
            "reference",
            "transaction id",
            "external id",
            "payment reference",
            "kenmerk",
        }
    ),
    "iban": frozenset(
        {"counterparty iban", "iban", "tegenrekening iban", "other account"}
    ),
}


def parse_bank_csv(
    content: bytes, *, filename: str
) -> tuple[tuple[ParsedCsvRow, ...], int]:
    """Parse a bounded CSV using semantic headers instead of provider positions."""

    if not content or len(content) > MAX_CSV_BYTES:
        raise CsvImportError("CSV must be non-empty and no larger than 5 MiB")
    if not Path(filename).name.lower().endswith((".csv", ".txt")):
        raise CsvImportError("upload must be a CSV or delimited text file")
    text = _decode(content)
    sample = text[:8192]
    try:
        dialect = csv.Sniffer().sniff(sample, delimiters=",;\t|")
    except csv.Error as error:
        raise CsvImportError("CSV delimiter could not be detected") from error
    reader = csv.DictReader(io.StringIO(text, newline=""), dialect=dialect)
    if not reader.fieldnames:
        raise CsvImportError("CSV must contain a header row")
    columns = _column_map(reader.fieldnames)
    if "date" not in columns or (
        "amount" not in columns and not ({"debit", "credit"} & columns.keys())
    ):
        raise CsvImportError("CSV needs recognizable date and amount columns")
    if "description" not in columns:
        raise CsvImportError("CSV needs a recognizable description column")
    rows: list[ParsedCsvRow] = []
    rejected = 0
    for line_number, raw in enumerate(reader, start=2):
        if line_number > MAX_CSV_ROWS + 1:
            raise CsvImportError("CSV contains more than 10000 data rows")
        try:
            rows.append(_parse_row(raw, columns=columns, line_number=line_number))
        except CsvImportError, ValueError:
            rejected += 1
    if not rows:
        raise CsvImportError("CSV contains no valid transaction rows")
    return tuple(rows), rejected


def detect_bank_label(filename: str, rows: tuple[ParsedCsvRow, ...]) -> str:
    """Return a display-only provider label without retaining the upload name."""

    normalized = _normalize_header(Path(filename).stem)
    labels = {
        "abn": "ABN AMRO",
        "revolut": "Revolut",
        "swedbank": "Swedbank",
        "trade republic": "Trade Republic",
        "traderepublic": "Trade Republic",
    }
    for marker, label in labels.items():
        if marker in normalized:
            return label
    descriptions = " ".join(row.description.casefold() for row in rows[:20])
    for marker, label in labels.items():
        if marker in descriptions:
            return label
    return "Connected bank CSV"


def csv_external_id(row: ParsedCsvRow, *, account_id: str, bank_label: str) -> str:
    """Build an opaque, deterministic identifier without leaking row content."""

    reference = row.external_reference or "|".join(
        (
            row.booked_on.isoformat(),
            format(row.amount, "f"),
            row.currency,
            _normalize_description(row.description),
            _normalize_iban(row.counterparty_iban or ""),
        )
    )
    digest = hashlib.sha256(
        f"v1|{bank_label.casefold()}|{account_id}|{reference}".encode()
    ).hexdigest()
    return f"pfa-csv-v1-{digest}"


def _decode(content: bytes) -> str:
    for encoding in ("utf-8-sig", "cp1252"):
        try:
            return content.decode(encoding)
        except UnicodeDecodeError:
            continue
    raise CsvImportError("CSV encoding is not supported")


def _normalize_header(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", value.casefold()).strip()


def _column_map(fieldnames: Iterable[str]) -> dict[str, str]:
    result: dict[str, str] = {}
    for original in fieldnames:
        normalized = _normalize_header(original)
        for role, aliases in HEADER_ALIASES.items():
            if normalized in aliases and role not in result:
                result[role] = original
    return result


def _parse_row(
    raw: dict[str, str | None], *, columns: dict[str, str], line_number: int
) -> ParsedCsvRow:
    def value(role: str) -> str:
        original = columns.get(role)
        return (raw.get(original) or "").strip() if original else ""

    amount_text = value("amount")
    if not amount_text:
        debit = value("debit")
        credit = value("credit")
        if bool(debit) == bool(credit):
            raise CsvImportError("row must contain exactly one debit or credit")
        amount_text = f"-{debit}" if debit else credit
    description = value("description")
    if not description:
        raise CsvImportError("row description is empty")
    return ParsedCsvRow(
        line_number=line_number,
        booked_on=_parse_date(value("date")),
        amount=_parse_decimal(amount_text),
        currency=value("currency") or "EUR",
        description=description,
        external_reference=value("reference") or None,
        counterparty_iban=value("iban") or None,
    )


def _parse_date(value: str) -> date:
    cleaned = value.strip()
    if "T" in cleaned:
        try:
            return datetime.fromisoformat(cleaned.replace("Z", "+00:00")).date()
        except ValueError:
            pass
    for pattern in ("%Y-%m-%d", "%d-%m-%Y", "%d/%m/%Y", "%Y/%m/%d", "%d.%m.%Y"):
        try:
            return datetime.strptime(cleaned, pattern).date()
        except ValueError:
            continue
    raise CsvImportError("transaction date is invalid")


def _parse_decimal(value: str) -> Decimal:
    cleaned = re.sub(r"[^0-9,.()\-+]", "", value.strip())
    negative_parentheses = cleaned.startswith("(") and cleaned.endswith(")")
    cleaned = cleaned.strip("()")
    if "," in cleaned and "." in cleaned:
        decimal_mark = "," if cleaned.rfind(",") > cleaned.rfind(".") else "."
        thousands_mark = "." if decimal_mark == "," else ","
        cleaned = cleaned.replace(thousands_mark, "").replace(decimal_mark, ".")
    elif "," in cleaned:
        tail = cleaned.rsplit(",", 1)[1]
        cleaned = cleaned.replace(",", "." if len(tail) <= 2 else "")
    try:
        result = Decimal(cleaned)
    except InvalidOperation as error:
        raise CsvImportError("transaction amount is invalid") from error
    return -abs(result) if negative_parentheses else result


def _one_account(
    accounts: tuple[FireflyAccount, ...], account_id: str, *, bank_label: str
) -> FireflyAccount:
    if account_id == "auto":
        marker = _normalize_header(bank_label)
        matches = tuple(
            account
            for account in accounts
            if marker != "connected bank csv"
            and marker in _normalize_header(account.attributes.name)
        )
    else:
        matches = tuple(account for account in accounts if account.id == account_id)
    if len(matches) != 1 or not matches[0].attributes.active:
        raise CsvImportConflictError(
            "bank account could not be detected uniquely; choose it explicitly"
        )
    return matches[0]


def _existing_fingerprints(
    existing: tuple[FireflyTransaction, ...], *, account_id: str
) -> tuple[
    Counter[tuple[date, Decimal, str, str]],
    set[str],
    Counter[tuple[date, Decimal, str]],
]:
    exact: Counter[tuple[date, Decimal, str, str]] = Counter()
    identifiers: set[str] = set()
    amounts: Counter[tuple[date, Decimal, str]] = Counter()
    for group in existing:
        for split in group.attributes.transactions:
            if split.source_id == account_id:
                signed_amount = -split.amount
            elif split.destination_id == account_id:
                signed_amount = split.amount
            else:
                continue
            signature = (split.date.date(), signed_amount, split.currency_code)
            amounts[signature] += 1
            exact[(*signature, _normalize_description(split.description))] += 1
            if split.external_id:
                identifiers.add(split.external_id)
    return exact, identifiers, amounts


def _classify_rows(
    rows: tuple[ParsedCsvRow, ...],
    *,
    account: FireflyAccount,
    existing: tuple[FireflyTransaction, ...],
    bank_label: str,
) -> tuple[CsvRowStatus, ...]:
    statuses: list[CsvRowStatus] = []
    seen_identifiers: set[str] = set()
    seen_unreferenced: Counter[tuple[date, Decimal, str, str]] = Counter()
    used_exact: Counter[tuple[date, Decimal, str, str]] = Counter()
    existing_exact, existing_identifiers, existing_amounts = _existing_fingerprints(
        existing, account_id=account.id
    )
    for row in rows:
        external_id = csv_external_id(row, account_id=account.id, bank_label=bank_label)
        exact_signature = (
            row.booked_on,
            row.amount,
            row.currency,
            _normalize_description(row.description),
        )
        amount_signature = (row.booked_on, row.amount, row.currency)
        provider_id = row.external_reference
        if external_id in existing_identifiers or (
            provider_id is not None and provider_id in existing_identifiers
        ):
            row_status = CsvRowStatus.DUPLICATE
        elif external_id in seen_identifiers:
            row_status = (
                CsvRowStatus.DUPLICATE
                if provider_id
                else CsvRowStatus.POSSIBLE_DUPLICATE
            )
        elif used_exact[exact_signature] < existing_exact[exact_signature]:
            used_exact[exact_signature] += 1
            row_status = CsvRowStatus.DUPLICATE
        elif existing_amounts[amount_signature] > 0:
            row_status = CsvRowStatus.POSSIBLE_DUPLICATE
        elif provider_id is None and seen_unreferenced[exact_signature] > 0:
            row_status = CsvRowStatus.POSSIBLE_DUPLICATE
        else:
            row_status = CsvRowStatus.NEW
        statuses.append(row_status)
        seen_identifiers.add(external_id)
        if provider_id is None:
            seen_unreferenced[exact_signature] += 1
    return tuple(statuses)


def _validate_account_currency(
    rows: tuple[ParsedCsvRow, ...], account: FireflyAccount
) -> None:
    if any(row.currency != account.attributes.currency_code for row in rows):
        raise CsvImportConflictError(
            "CSV currency does not match the selected Firefly account"
        )


def _normalize_description(value: str) -> str:
    return " ".join(value.casefold().split())


def _normalize_iban(value: str) -> str:
    return re.sub(r"\s+", "", value).upper()
