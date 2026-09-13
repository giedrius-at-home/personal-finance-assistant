"""Single-process, privacy-safe Firefly polling for derived dashboard state."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Mapping
from contextlib import AsyncExitStack
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from decimal import Decimal, InvalidOperation
from zoneinfo import ZoneInfo

from pydantic import SecretStr

from finance_assistant.alert_evaluation import AlertEvaluationService
from finance_assistant.domain.alerting import AlertPolicy
from finance_assistant.domain.canonical import SourceSystem
from finance_assistant.domain.cash_overview import payday_window
from finance_assistant.domain.models import BudgetPolicy, Money
from finance_assistant.ingestion.budgets import BudgetCashContext, build_budget_windows
from finance_assistant.ingestion.mapping import (
    AccountLink,
    CurrencyRegistry,
    MappingError,
)
from finance_assistant.ingestion.service import IngestionResult, IngestionService
from finance_assistant.integrations.firefly import FireflyClient, FireflyTransaction
from finance_assistant.integrations.ghostfolio import GhostfolioClient
from finance_assistant.notifications.macos import MacOSNotifier
from finance_assistant.notifications.service import AlertDispatcher
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.subscriptions import (
    SubscriptionSnapshot,
    subscription_report,
)

# Uvicorn installs a dedicated handler and does not configure arbitrary module
# loggers. Reuse its error logger so content-free poll outcomes are observable.
LOGGER = logging.getLogger("uvicorn.error")
DEFAULT_SYNC_INTERVAL_SECONDS = 86_400
MIN_SYNC_INTERVAL_SECONDS = 300
DEFAULT_FAILURE_RETRY_SECONDS = 300
MAX_FAILURE_RETRY_SECONDS = 3_600
DEFAULT_INCREMENTAL_LOOKBACK_DAYS = 7
DEFAULT_FULL_REFRESH_DAYS = 30
EARLIEST_TRANSACTION_DATE = date(1970, 1, 1)


@dataclass(frozen=True, slots=True)
class FireflySyncSettings:
    """Validated non-secret settings plus an opaque Firefly credential."""

    base_url: str
    token: SecretStr
    interval_seconds: int = DEFAULT_SYNC_INTERVAL_SECONDS
    incremental_lookback_days: int = DEFAULT_INCREMENTAL_LOOKBACK_DAYS
    full_refresh_days: int = DEFAULT_FULL_REFRESH_DAYS
    minimum_buffer_eur: Decimal = Decimal(0)

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflySyncSettings | None:
        enabled = environment.get("PFA_FIREFLY_SYNC_ENABLED", "false").casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_FIREFLY_SYNC_ENABLED must be true or false")
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError("PFA_FIREFLY_API_TOKEN is required when sync is enabled")
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError("PFA_FIREFLY_BASE_URL is required when sync is enabled")
        try:
            interval = int(
                environment.get(
                    "PFA_FIREFLY_SYNC_INTERVAL_SECONDS",
                    str(DEFAULT_SYNC_INTERVAL_SECONDS),
                )
            )
            lookback = int(
                environment.get(
                    "PFA_FIREFLY_INCREMENTAL_LOOKBACK_DAYS",
                    str(DEFAULT_INCREMENTAL_LOOKBACK_DAYS),
                )
            )
            full_refresh = int(
                environment.get(
                    "PFA_FIREFLY_FULL_REFRESH_DAYS",
                    str(DEFAULT_FULL_REFRESH_DAYS),
                )
            )
        except ValueError as error:
            raise ValueError(
                "each Firefly synchronization interval must be an integer"
            ) from error
        try:
            minimum_buffer_eur = Decimal(
                environment.get("PFA_MINIMUM_CASH_BUFFER_EUR", "0")
            )
        except InvalidOperation as error:
            raise ValueError("minimum EUR cash buffer must be a number") from error
        if interval < MIN_SYNC_INTERVAL_SECONDS:
            raise ValueError(
                f"PFA_FIREFLY_SYNC_INTERVAL_SECONDS must be at least "
                f"{MIN_SYNC_INTERVAL_SECONDS}"
            )
        if not 1 <= lookback <= 90:
            raise ValueError("incremental lookback must be between 1 and 90 days")
        if not 1 <= full_refresh <= 365:
            raise ValueError("full refresh must be between 1 and 365 days")
        if not minimum_buffer_eur.is_finite() or minimum_buffer_eur < 0:
            raise ValueError("minimum EUR cash buffer must be finite and non-negative")
        return cls(
            base_url=base_url,
            token=token,
            interval_seconds=interval,
            incremental_lookback_days=lookback,
            full_refresh_days=full_refresh,
            minimum_buffer_eur=minimum_buffer_eur,
        )


@dataclass(frozen=True, slots=True)
class FireflyCategoryWriteSettings:
    """Opt-in credentials for reviewed category-only writes."""

    base_url: str
    token: SecretStr

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflyCategoryWriteSettings | None:
        enabled = environment.get(
            "PFA_FIREFLY_CATEGORY_WRITES_ENABLED", "false"
        ).casefold()
        if enabled not in {"true", "false"}:
            raise ValueError(
                "PFA_FIREFLY_CATEGORY_WRITES_ENABLED must be true or false"
            )
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError(
                "PFA_FIREFLY_API_TOKEN is required when category writes are enabled"
            )
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError(
                "PFA_FIREFLY_BASE_URL is required when category writes are enabled"
            )
        return cls(base_url=base_url, token=token)


@dataclass(frozen=True, slots=True)
class FireflyCsvImportSettings:
    """Opt-in credentials for previewed, owner-approved CSV imports."""

    base_url: str
    token: SecretStr

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflyCsvImportSettings | None:
        enabled = environment.get("PFA_FIREFLY_CSV_IMPORTS_ENABLED", "false").casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_FIREFLY_CSV_IMPORTS_ENABLED must be true or false")
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError(
                "PFA_FIREFLY_API_TOKEN is required when CSV imports are enabled"
            )
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError(
                "PFA_FIREFLY_BASE_URL is required when CSV imports are enabled"
            )
        return cls(base_url=base_url, token=token)


@dataclass(frozen=True, slots=True)
class GhostfolioSyncSettings:
    """Opt-in read-only Ghostfolio sync with explicit mirror ownership."""

    base_url: str
    bearer_token: SecretStr
    links: tuple[AccountLink, ...] = ()

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> GhostfolioSyncSettings | None:
        enabled = environment.get("PFA_GHOSTFOLIO_SYNC_ENABLED", "false").casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_GHOSTFOLIO_SYNC_ENABLED must be true or false")
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_GHOSTFOLIO_BEARER_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError(
                "PFA_GHOSTFOLIO_BEARER_TOKEN is required when sync is enabled"
            )
        base_url = environment.get("PFA_GHOSTFOLIO_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError("PFA_GHOSTFOLIO_BASE_URL is required when sync is enabled")
        links: list[AccountLink] = []
        raw_links = environment.get("PFA_GHOSTFOLIO_ACCOUNT_LINKS", "").strip()
        for raw_link in filter(None, (item.strip() for item in raw_links.split(","))):
            parts = tuple(item.strip() for item in raw_link.split(":"))
            if len(parts) != 3:
                raise ValueError(
                    "PFA_GHOSTFOLIO_ACCOUNT_LINKS entries must have three fields"
                )
            try:
                owner = SourceSystem(parts[2])
            except ValueError as error:
                raise ValueError(
                    "PFA_GHOSTFOLIO_ACCOUNT_LINKS has an invalid owner"
                ) from error
            links.append(AccountLink(parts[0], parts[1], owner=owner))
        return cls(base_url=base_url, bearer_token=token, links=tuple(links))


class FireflySyncRunner:
    """Fetch one read-only snapshot into the gateway-owned SQLite connection."""

    def __init__(
        self,
        *,
        state: SQLiteDerivedState,
        settings: FireflySyncSettings,
        ghostfolio_settings: GhostfolioSyncSettings | None = None,
    ) -> None:
        self._state = state
        self._settings = settings
        self._ghostfolio_settings = ghostfolio_settings

    async def __call__(self) -> IngestionResult:
        full_snapshot = False
        try:
            async with AsyncExitStack() as stack:
                firefly = await stack.enter_async_context(
                    FireflyClient(
                        base_url=f"{self._settings.base_url}/api/v1",
                        token=self._settings.token,
                    )
                )
                ghostfolio = None
                links: tuple[AccountLink, ...] = ()
                if self._ghostfolio_settings is not None:
                    ghostfolio = await stack.enter_async_context(
                        GhostfolioClient(
                            base_url=f"{self._ghostfolio_settings.base_url}/api/v1",
                            bearer_token=self._ghostfolio_settings.bearer_token,
                        )
                    )
                    links = self._ghostfolio_settings.links
                now = datetime.now(UTC)
                today = now.astimezone(
                    ZoneInfo(self._state.get_settings().timezone)
                ).date()
                start, end = today - timedelta(days=31), today + timedelta(days=35)
                subscriptions = SubscriptionSnapshot(
                    fetched_at=now,
                    start=start,
                    end=end,
                    bills=await firefly.list_bills(start=start, end=end),
                )
                last_full = self._state.last_full_firefly_sync()
                full_snapshot = last_full is None or now - last_full >= timedelta(
                    days=self._settings.full_refresh_days
                )
                if full_snapshot:
                    transactions = await _complete_transaction_snapshot(
                        firefly, through=today
                    )
                else:
                    transactions = await firefly.list_transactions(
                        start=today
                        - timedelta(days=self._settings.incremental_lookback_days),
                        end=today,
                    )
                result = await IngestionService(
                    firefly=firefly,
                    ghostfolio=ghostfolio,
                    state=self._state,
                    links=links,
                    transactions=transactions,
                    complete_transactions=full_snapshot,
                    subscriptions=subscriptions,
                ).ingest()
                try:
                    await _evaluate_alerts(
                        firefly,
                        state=self._state,
                        subscriptions=subscriptions,
                        now=now,
                        minimum_buffer_eur=self._settings.minimum_buffer_eur,
                    )
                except Exception as error:  # noqa: BLE001 - optional derived alerts
                    LOGGER.error(
                        "Alert evaluation failed error_type=%s", type(error).__name__
                    )
        except Exception:
            self._state.record_firefly_sync(success=False, now=datetime.now(UTC))
            raise
        self._state.record_firefly_sync(
            success=True,
            now=datetime.now(UTC),
            full_snapshot=full_snapshot,
        )
        return result


async def _complete_transaction_snapshot(
    firefly: FireflyClient, *, through: date
) -> tuple[FireflyTransaction, ...]:
    """Fetch bounded calendar slices so lifetime history cannot hit one page cap."""

    transactions: list[FireflyTransaction] = []
    start = EARLIEST_TRANSACTION_DATE
    while start <= through:
        end = min(date(start.year, 12, 31), through)
        transactions.extend(await firefly.list_transactions(start=start, end=end))
        start = date(start.year + 1, 1, 1)
    return tuple(transactions)


async def _evaluate_alerts(
    firefly: FireflyClient,
    *,
    state: SQLiteDerivedState,
    subscriptions: SubscriptionSnapshot,
    now: datetime,
    minimum_buffer_eur: Decimal,
) -> None:
    """Refresh deterministic Firefly budget and goal alert evidence."""

    settings = state.get_settings()
    zone = ZoneInfo(settings.timezone)
    today = now.astimezone(zone).date()
    month_start = today.replace(day=1)
    next_month = (
        date(today.year + 1, 1, 1)
        if today.month == 12
        else date(today.year, today.month + 1, 1)
    )
    limits = await firefly.list_budget_limits(
        start=month_start, end=next_month - timedelta(days=1)
    )
    accounts, transactions, goals, _observed = state.read_projection_snapshot()
    currencies = tuple(
        sorted(
            {account.currency for account in accounts},
            key=lambda currency: (currency.code, currency.minor_unit),
        )
    )
    registry = CurrencyRegistry(currencies)
    next_payday = (
        payday_window(today, settings.payday_day)[1]
        if settings.payday_day is not None
        else None
    )
    subscriptions_due = subscription_report(
        subscriptions,
        transactions,
        now=now,
        timezone=settings.timezone,
        next_payday=next_payday,
    )
    available: dict[str, Decimal] = {}
    for account in accounts:
        if account.canonical_owner.system is SourceSystem.FIREFLY_III:
            code = account.currency.code
            available[code] = available.get(code, Decimal()) + account.balance.amount
    upcoming = {
        cost.currency.code: cost.amount for cost in subscriptions_due.expected_costs
    }
    planned: dict[str, Decimal] = {}
    for goal in goals:
        code = goal.protected_amount.currency.code
        planned[code] = planned.get(code, Decimal()) + goal.protected_amount.amount
    contexts: dict[str, BudgetCashContext] = {}
    for currency in currencies:
        if currency.code not in available:
            continue
        contexts[currency.code] = BudgetCashContext(
            available_cash=Money(
                amount=max(available[currency.code], Decimal()), currency=currency
            ),
            upcoming_committed=Money(
                amount=upcoming.get(currency.code, Decimal()), currency=currency
            ),
            planned_savings=Money(
                amount=planned.get(currency.code, Decimal()), currency=currency
            ),
            minimum_buffer=Money(
                amount=(minimum_buffer_eur if currency.code == "EUR" else Decimal()),
                currency=currency,
            ),
        )
    try:
        windows = build_budget_windows(
            limits,
            as_of=today,
            currencies=registry,
            cash_by_currency=contexts,
        )
    except MappingError as error:
        LOGGER.error("Budget mapping failed error_type=%s", type(error).__name__)
        windows = ()
    policy = AlertPolicy(
        version=settings.policy_version,
        budget=BudgetPolicy(
            version=settings.policy_version,
            heads_up_spent_ratio=settings.heads_up_spent_ratio,
            heads_up_pace_ratio=settings.heads_up_pace_ratio,
        ),
        heads_up_cooldown_seconds=settings.heads_up_cooldown_hours * 3_600,
        warning_cooldown_seconds=settings.warning_cooldown_hours * 3_600,
        critical_cooldown_seconds=settings.critical_cooldown_hours * 3_600,
    )
    dispatcher = AlertDispatcher(
        state=state,
        notifier=MacOSNotifier(enabled=False),
        policy=policy,
    )
    AlertEvaluationService(
        state=state,
        dispatcher=dispatcher,
        policy=policy,
        timezone_name=settings.timezone,
    ).evaluate(now=now, budgets=windows)


async def run_firefly_sync_loop(
    sync_once: Callable[[], Awaitable[IngestionResult]],
    *,
    interval_seconds: int,
    failure_retry_seconds: int = DEFAULT_FAILURE_RETRY_SECONDS,
    max_failure_retry_seconds: int = MAX_FAILURE_RETRY_SECONDS,
) -> None:
    """Run immediately, back off on failure, and resume the normal cadence."""

    retry_delay = failure_retry_seconds
    while True:
        try:
            result = await sync_once()
        except asyncio.CancelledError:
            raise
        except Exception as error:  # noqa: BLE001 - isolate an optional poller
            LOGGER.error("Firefly sync failed error_type=%s", type(error).__name__)
            delay = retry_delay
            retry_delay = min(retry_delay * 2, max_failure_retry_seconds)
        else:
            LOGGER.info(
                "Firefly sync completed accounts=%d transactions=%d "
                "goals=%d changed=%d",
                result.accounts_seen,
                result.transactions_seen,
                result.goals_seen,
                result.projections_changed,
            )
            delay = interval_seconds
            retry_delay = failure_retry_seconds
        await asyncio.sleep(delay)
