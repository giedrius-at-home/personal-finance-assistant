"""HTTP boundary for deterministic financial calculations."""

import asyncio
import os
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import asynccontextmanager
from datetime import date
from pathlib import Path

from fastapi import APIRouter, FastAPI, HTTPException, Request, status
from fastapi.staticfiles import StaticFiles
from starlette.middleware.base import RequestResponseEndpoint
from starlette.responses import PlainTextResponse, Response

from finance_assistant import __version__
from finance_assistant.categorisation_runtime import (
    CategorySuggestionRunner,
    CategorySuggestionSettings,
)
from finance_assistant.csv_import import CsvImportCoordinator
from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import (
    BudgetEvaluation,
    BudgetWindow,
    GoalEvaluation,
    SavingsGoal,
)
from finance_assistant.ingestion.runtime import (
    FireflyCategoryWriteSettings,
    FireflyCsvImportSettings,
    FireflySyncRunner,
    FireflySyncSettings,
    run_firefly_sync_loop,
)
from finance_assistant.ingestion.service import IngestionResult
from finance_assistant.integrations.firefly import (
    FireflyCategoryClient,
    FireflyCsvImportClient,
)
from finance_assistant.integrations.firefly_webhook import (
    MAX_WEBHOOK_BYTES,
    FireflyWebhookReceiver,
    ReplayCapacityError,
    WebhookAuthenticationError,
    WebhookPayloadError,
)
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.web import install_web_routes, new_csrf_token


def create_app(
    *,
    webhook_secret: str | None = None,
    webhook_clock: Callable[[], int] | None = None,
    database_path: Path | str | None = None,
    demo_mode: bool = False,
    allowed_hosts: frozenset[str] = frozenset({"127.0.0.1", "localhost", "::1"}),
    firefly_sync_settings: FireflySyncSettings | None = None,
    firefly_category_write_settings: FireflyCategoryWriteSettings | None = None,
    firefly_csv_import_settings: FireflyCsvImportSettings | None = None,
    category_suggestion_settings: CategorySuggestionSettings | None = None,
    category_updater: (
        Callable[[str, str | None, str | None], Awaitable[None]] | None
    ) = None,
    csv_import_coordinator: CsvImportCoordinator | None = None,
    owner_name_aliases: tuple[str, ...] = (),
) -> FastAPI:
    """Build the API with an explicitly configured webhook boundary."""

    state = SQLiteDerivedState(database_path) if database_path else None
    if state is not None:
        state.migrate_up()
    suggestion_runner = (
        CategorySuggestionRunner(state=state, settings=category_suggestion_settings)
        if state is not None and category_suggestion_settings is not None
        else None
    )
    sync_runner = (
        FireflySyncRunner(state=state, settings=firefly_sync_settings)
        if state is not None and firefly_sync_settings is not None
        else None
    )
    sync_lock = asyncio.Lock()

    async def sync_and_suggest() -> IngestionResult:
        if sync_runner is None:
            raise RuntimeError("Firefly sync is disabled")
        async with sync_lock:
            result = await sync_runner()
            if suggestion_runner is not None:
                await suggestion_runner()
            return result

    configured_csv_importer = csv_import_coordinator
    if configured_csv_importer is None and firefly_csv_import_settings is not None:
        configured_csv_importer = CsvImportCoordinator(
            client_factory=lambda: FireflyCsvImportClient(
                base_url=f"{firefly_csv_import_settings.base_url}/api/v1",
                token=firefly_csv_import_settings.token,
            )
        )

    async def configured_category_updater(
        transaction_id: str,
        expected_category: str | None,
        category: str | None,
    ) -> None:
        if firefly_category_write_settings is None:
            raise RuntimeError("reviewed Firefly category writes are disabled")
        async with FireflyCategoryClient(
            base_url=f"{firefly_category_write_settings.base_url}/api/v1",
            token=firefly_category_write_settings.token,
        ) as client:
            await client.set_category(
                transaction_id=transaction_id,
                expected_category=expected_category,
                category=category,
            )

    @asynccontextmanager
    async def lifespan(_application: FastAPI) -> AsyncIterator[None]:
        sync_task: asyncio.Task[None] | None = None
        if sync_runner is not None and firefly_sync_settings is not None:
            sync_task = asyncio.create_task(
                run_firefly_sync_loop(
                    sync_and_suggest,
                    interval_seconds=firefly_sync_settings.interval_seconds,
                )
            )
        try:
            yield
        finally:
            if sync_task is not None:
                sync_task.cancel()
                try:
                    await sync_task
                except asyncio.CancelledError:
                    pass
            if state is not None:
                state.close()

    application = FastAPI(
        title="Personal Finance Assistant",
        description="Local deterministic analytics for Firefly III and Ghostfolio.",
        version=__version__,
        lifespan=lifespan,
    )
    application.mount(
        "/static",
        StaticFiles(directory=Path(__file__).parent / "static"),
        name="static",
    )
    application.state.firefly_webhook_receiver = (
        FireflyWebhookReceiver(
            secret=webhook_secret,
            clock=webhook_clock,
            replay_guard=state,
        )
        if webhook_secret
        else None
    )

    @application.middleware("http")
    async def local_security(
        request: Request, call_next: RequestResponseEndpoint
    ) -> Response:
        host = request.url.hostname or ""
        test_client = request.client is not None and request.client.host == "testclient"
        if host not in allowed_hosts and not (test_client and host == "testserver"):
            return PlainTextResponse("invalid host", status_code=400)
        response = await call_next(request)
        response.headers["Cache-Control"] = "no-store"
        response.headers["Content-Security-Policy"] = (
            "default-src 'self'; style-src 'self'; img-src 'self'; "
            "form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
        )
        response.headers["Referrer-Policy"] = "no-referrer"
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["X-Frame-Options"] = "DENY"
        return response

    application.add_api_route("/health", health, methods=["GET"], tags=["system"])
    application.add_api_route(
        "/v1/goals/evaluate",
        goal_evaluation,
        methods=["POST"],
        tags=["goals"],
    )
    application.add_api_route(
        "/v1/budgets/evaluate",
        budget_evaluation,
        methods=["POST"],
        tags=["budgets"],
    )
    application.add_api_route(
        "/v1/webhooks/firefly",
        receive_firefly_webhook,
        methods=["POST"],
        tags=["ingestion"],
        status_code=status.HTTP_202_ACCEPTED,
    )
    web_router = APIRouter()
    install_web_routes(
        web_router,
        state=state,
        demo_mode=demo_mode,
        csrf_token=new_csrf_token(),
        owner_name_aliases=owner_name_aliases,
        category_updater=(
            category_updater
            or (
                configured_category_updater
                if firefly_category_write_settings is not None
                else None
            )
        ),
        suggestion_runner=suggestion_runner,
        csv_import_coordinator=configured_csv_importer,
        csv_post_import_sync=sync_and_suggest if sync_runner is not None else None,
    )
    application.include_router(web_router)
    return application


def health() -> dict[str, str]:
    """Return process health without revealing configuration or financial data."""

    return {"status": "ok", "version": __version__}


def goal_evaluation(goal: SavingsGoal, as_of: date | None = None) -> GoalEvaluation:
    """Evaluate savings progress as of a supplied date or today."""

    return evaluate_goal(goal, as_of=as_of or date.today())


def budget_evaluation(window: BudgetWindow) -> BudgetEvaluation:
    """Evaluate spending pace, forecast, and safe-to-spend amount."""

    return evaluate_budget(window)


async def receive_firefly_webhook(request: Request) -> dict[str, str]:
    """Authenticate raw bytes before parsing and acknowledge replay safely."""

    receiver: FireflyWebhookReceiver | None = request.app.state.firefly_webhook_receiver
    if receiver is None:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="webhook receiver is not configured",
        )
    content_length = request.headers.get("content-length")
    if content_length is not None:
        try:
            declared_length = int(content_length)
        except ValueError as error:
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail="invalid content length",
            ) from error
        if declared_length < 0 or declared_length > MAX_WEBHOOK_BYTES:
            raise HTTPException(
                status_code=status.HTTP_413_CONTENT_TOO_LARGE,
                detail="webhook payload is too large",
            )
    body = bytearray()
    async for chunk in request.stream():
        if len(body) + len(chunk) > MAX_WEBHOOK_BYTES:
            raise HTTPException(
                status_code=status.HTTP_413_CONTENT_TOO_LARGE,
                detail="webhook payload is too large",
            )
        body.extend(chunk)
    raw_body = bytes(body)
    try:
        accepted = receiver.receive(raw_body, request.headers.get("signature", ""))
    except WebhookAuthenticationError as error:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="webhook authentication failed",
        ) from error
    except WebhookPayloadError as error:
        raise HTTPException(
            status_code=status.HTTP_422_UNPROCESSABLE_CONTENT,
            detail="invalid webhook payload",
        ) from error
    except ReplayCapacityError as error:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="webhook receipt is temporarily unavailable",
        ) from error
    return {"status": "accepted" if accepted else "duplicate"}


_sync_settings = FireflySyncSettings.from_environment(os.environ)
_category_write_settings = FireflyCategoryWriteSettings.from_environment(os.environ)
_csv_import_settings = FireflyCsvImportSettings.from_environment(os.environ)
_category_suggestion_settings = CategorySuggestionSettings.from_environment(os.environ)


def _owner_name_aliases(raw: str | None) -> tuple[str, ...]:
    if not raw:
        return ()
    aliases = tuple(item.strip() for item in raw.split(",") if item.strip())
    if len(aliases) > 16 or any(len(item) > 100 for item in aliases):
        raise ValueError(
            "PFA_OWNER_NAME_ALIASES contains too many or oversized aliases"
        )
    return tuple(dict.fromkeys(aliases))


app = create_app(
    webhook_secret=os.getenv("PFA_FIREFLY_WEBHOOK_SECRET"),
    database_path=os.getenv("PFA_DATABASE_PATH"),
    demo_mode=os.getenv("PFA_DEMO_MODE", "false").casefold() == "true",
    firefly_sync_settings=_sync_settings,
    firefly_category_write_settings=_category_write_settings,
    firefly_csv_import_settings=_csv_import_settings,
    category_suggestion_settings=_category_suggestion_settings,
    owner_name_aliases=_owner_name_aliases(os.getenv("PFA_OWNER_NAME_ALIASES")),
)
