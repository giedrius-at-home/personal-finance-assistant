"""Secure loopback-only HTML review and settings routes."""

from __future__ import annotations

import hmac
import secrets
from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path
from urllib.parse import parse_qs

from fastapi import APIRouter, HTTPException, Request, status
from fastapi.responses import HTMLResponse, RedirectResponse
from fastapi.templating import Jinja2Templates
from pydantic import TypeAdapter, ValidationError

from finance_assistant.categorisation_service import ProposalStatus
from finance_assistant.dashboard import DashboardService, DashboardView
from finance_assistant.domain.models import Currency
from finance_assistant.domain.settings import OwnerSettings, SettingsPreview
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)

MAX_FORM_BYTES = 8_192
TEMPLATES = Jinja2Templates(directory=Path(__file__).parent / "templates")


def format_money(amount: Decimal, currency: Currency) -> str:
    """Render exact money using the currency's declared minor unit."""

    normalized = abs(amount) if amount == 0 else amount
    return f"{normalized:.{currency.minor_unit}f}"


TEMPLATES.env.filters["money"] = format_money


def install_web_routes(
    router: APIRouter,
    *,
    state: SQLiteDerivedState | None,
    demo_mode: bool,
    csrf_token: str,
    owner_name_aliases: tuple[str, ...] = (),
) -> None:
    service = DashboardService(
        state, demo_mode=demo_mode, owner_name_aliases=owner_name_aliases
    )

    @router.get("/", include_in_schema=False)
    async def index() -> RedirectResponse:
        return RedirectResponse(
            "/dashboard", status_code=status.HTTP_307_TEMPORARY_REDIRECT
        )

    @router.get("/dashboard", response_class=HTMLResponse, include_in_schema=False)
    async def dashboard(request: Request) -> HTMLResponse:
        response = TEMPLATES.TemplateResponse(
            request=request,
            name="dashboard.html",
            context={
                "view": service.view(now=datetime.now(UTC)),
                "csrf_token": csrf_token,
            },
        )
        response.set_cookie(
            "pfa_csrf",
            csrf_token,
            httponly=True,
            samesite="strict",
            secure=False,
        )
        return response

    @router.get("/v1/dashboard", response_model=DashboardView, tags=["review"])
    async def dashboard_json() -> DashboardView:
        return service.view(now=datetime.now(UTC))

    @router.post("/review/{proposal_id}/{decision}", include_in_schema=False)
    async def review(
        proposal_id: str, decision: str, request: Request
    ) -> RedirectResponse:
        values = await _verified_form(request, csrf_token)
        if state is None or demo_mode:
            raise HTTPException(
                status_code=409, detail="demo and empty-state reviews are read-only"
            )
        if decision == "approve":
            token = state.preview_batch((proposal_id,))
            state.approve_batch((proposal_id,), preview_token=token)
        elif decision == "reject":
            if not state.resolve_proposal(proposal_id, status=ProposalStatus.REJECTED):
                raise HTTPException(
                    status_code=409, detail="proposal is no longer pending"
                )
        elif decision == "correct":
            category = _one(values, "category", max_length=128)
            proposal = state.get_pending(proposal_id)
            if proposal is None:
                raise HTTPException(
                    status_code=409, detail="proposal is no longer pending"
                )
            from finance_assistant.categorisation_service import source_identity
            from finance_assistant.domain.categorisation import CalibrationExample

            state.record_correction(
                CalibrationExample(
                    source_identity=source_identity(proposal.source),
                    feature_hash=proposal.feature_hash,
                    normalized_merchant=proposal.normalized_merchant,
                    approved_category=category,
                )
            )
            state.resolve_proposal(proposal_id, status=ProposalStatus.CORRECTED)
        else:
            raise HTTPException(status_code=404, detail="unknown review decision")
        return RedirectResponse("/dashboard", status_code=status.HTTP_303_SEE_OTHER)

    @router.post(
        "/settings/preview", response_class=HTMLResponse, include_in_schema=False
    )
    async def settings_preview(request: Request) -> HTMLResponse:
        values = await _verified_form(request, csrf_token)
        if state is None:
            raise HTTPException(
                status_code=409, detail="settings require persistent local state"
            )
        desired = _settings(values)
        preview = state.preview_settings(desired)
        return TEMPLATES.TemplateResponse(
            request=request,
            name="settings_preview.html",
            context={"preview": preview, "csrf_token": csrf_token},
        )

    @router.post("/settings/apply", include_in_schema=False)
    async def settings_apply(request: Request) -> RedirectResponse:
        values = await _verified_form(request, csrf_token)
        if state is None:
            raise HTTPException(
                status_code=409, detail="settings require persistent local state"
            )
        desired = _settings(values)
        preview = SettingsPreview(
            token=_one(values, "preview_token", max_length=64),
            before=state.get_settings(),
            after=desired,
        )
        try:
            state.apply_settings(preview)
        except PersistenceConflictError as error:
            raise HTTPException(status_code=409, detail=str(error)) from error
        return RedirectResponse("/dashboard", status_code=status.HTTP_303_SEE_OTHER)

    @router.post("/settings/{revision_id}/rollback", include_in_schema=False)
    async def settings_rollback(revision_id: int, request: Request) -> RedirectResponse:
        await _verified_form(request, csrf_token)
        if state is None or not state.rollback_settings(revision_id):
            raise HTTPException(
                status_code=409, detail="revision cannot be rolled back"
            )
        return RedirectResponse("/dashboard", status_code=status.HTTP_303_SEE_OTHER)


async def _verified_form(request: Request, expected_token: str) -> dict[str, list[str]]:
    content_type = request.headers.get("content-type", "").split(";", 1)[0].strip()
    if content_type != "application/x-www-form-urlencoded":
        raise HTTPException(status_code=415, detail="form encoding required")
    body = await request.body()
    if len(body) > MAX_FORM_BYTES:
        raise HTTPException(status_code=413, detail="form is too large")
    try:
        values = parse_qs(body.decode("utf-8"), strict_parsing=True, max_num_fields=24)
    except (UnicodeDecodeError, ValueError) as error:
        raise HTTPException(status_code=400, detail="invalid form") from error
    supplied = _one(values, "csrf_token", max_length=128)
    cookie = request.cookies.get("pfa_csrf", "")
    if not hmac.compare_digest(supplied, expected_token) or not hmac.compare_digest(
        cookie, expected_token
    ):
        raise HTTPException(status_code=403, detail="CSRF validation failed")
    origin = request.headers.get("origin", "")
    fetch_site = request.headers.get("sec-fetch-site", "")
    origin_matches = origin.rstrip("/") == str(request.base_url).rstrip("/")
    if not origin_matches and fetch_site != "same-origin":
        raise HTTPException(status_code=403, detail="same-origin request required")
    return values


def _one(values: dict[str, list[str]], key: str, *, max_length: int) -> str:
    found = values.get(key, [])
    if len(found) != 1 or not found[0] or len(found[0]) > max_length:
        raise HTTPException(status_code=422, detail=f"invalid {key}")
    return found[0]


def _settings(values: dict[str, list[str]]) -> OwnerSettings:
    payload = {
        "timezone": _one(values, "timezone", max_length=64),
        "evaluation_hour": _one(values, "evaluation_hour", max_length=2),
        "notifications_enabled": values.get("notifications_enabled") == ["on"],
        "heads_up_spent_ratio": _one(values, "heads_up_spent_ratio", max_length=8),
        "heads_up_pace_ratio": _one(values, "heads_up_pace_ratio", max_length=8),
        "heads_up_cooldown_hours": _one(
            values, "heads_up_cooldown_hours", max_length=3
        ),
        "warning_cooldown_hours": _one(values, "warning_cooldown_hours", max_length=3),
        "critical_cooldown_hours": _one(
            values, "critical_cooldown_hours", max_length=3
        ),
    }
    try:
        return TypeAdapter(OwnerSettings).validate_python(payload)
    except ValidationError as error:
        raise HTTPException(status_code=422, detail="invalid settings") from error


def new_csrf_token() -> str:
    return secrets.token_urlsafe(32)
