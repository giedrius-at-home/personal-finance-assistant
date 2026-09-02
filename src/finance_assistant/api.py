"""HTTP boundary for deterministic financial calculations."""

import os
from collections.abc import AsyncIterator, Callable
from contextlib import asynccontextmanager
from datetime import date
from pathlib import Path

from fastapi import APIRouter, FastAPI, HTTPException, Request, status
from fastapi.staticfiles import StaticFiles
from starlette.middleware.base import RequestResponseEndpoint
from starlette.responses import PlainTextResponse, Response

from finance_assistant import __version__
from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import (
    BudgetEvaluation,
    BudgetWindow,
    GoalEvaluation,
    SavingsGoal,
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
) -> FastAPI:
    """Build the API with an explicitly configured webhook boundary."""

    state = SQLiteDerivedState(database_path) if database_path else None
    if state is not None:
        state.migrate_up()

    @asynccontextmanager
    async def lifespan(_application: FastAPI) -> AsyncIterator[None]:
        yield
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


app = create_app(
    webhook_secret=os.getenv("PFA_FIREFLY_WEBHOOK_SECRET"),
    database_path=os.getenv("PFA_DATABASE_PATH"),
    demo_mode=os.getenv("PFA_DEMO_MODE", "false").casefold() == "true",
)
