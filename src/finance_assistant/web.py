"""Secure loopback-only HTML review and settings routes."""

from __future__ import annotations

import hmac
import secrets
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path
from typing import Annotated
from urllib.parse import parse_qs

from fastapi import APIRouter, File, Form, HTTPException, Request, UploadFile, status
from fastapi.responses import HTMLResponse, RedirectResponse
from fastapi.templating import Jinja2Templates
from pydantic import TypeAdapter, ValidationError

from finance_assistant.categorisation_service import ProposalStatus
from finance_assistant.csv_import import (
    MAX_CSV_BYTES,
    CsvImportConflictError,
    CsvImportCoordinator,
    CsvImportError,
)
from finance_assistant.dashboard import DashboardService, DashboardView
from finance_assistant.domain.canonical import (
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import Currency
from finance_assistant.domain.settings import OwnerSettings, SettingsPreview
from finance_assistant.integrations._http import UpstreamError
from finance_assistant.integrations.firefly import (
    FireflyCategoryConflictError,
    FireflyCategoryWriteError,
    FireflyCsvImportWriteError,
)
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)

CategoryUpdater = Callable[[str, str | None, str | None], Awaitable[None]]
SuggestionRunner = Callable[[], Awaitable[int]]

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
    category_updater: CategoryUpdater | None = None,
    suggestion_runner: SuggestionRunner | None = None,
    csv_import_coordinator: CsvImportCoordinator | None = None,
    csv_post_import_sync: Callable[[], Awaitable[object]] | None = None,
) -> None:
    service = DashboardService(
        state,
        demo_mode=demo_mode,
        owner_name_aliases=owner_name_aliases,
        upstream_writes=(
            "reviewed categories only" if category_updater is not None else "disabled"
        ),
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
                "csv_import_enabled": csv_import_coordinator is not None
                and not demo_mode,
                "csv_import_accounts": state.list_accounts() if state else (),
                "csv_import_status": _import_status(request),
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

    @router.post("/review/suggest", include_in_schema=False)
    async def suggest(request: Request) -> RedirectResponse:
        await _verified_form(request, csrf_token)
        if state is None or demo_mode or suggestion_runner is None:
            raise HTTPException(
                status_code=409, detail="local category suggestions are disabled"
            )
        await suggestion_runner()
        return RedirectResponse("/dashboard", status_code=status.HTTP_303_SEE_OTHER)

    @router.post(
        "/imports/csv/preview", response_class=HTMLResponse, include_in_schema=False
    )
    async def csv_preview(
        request: Request,
        account_id: Annotated[str, Form(min_length=1, max_length=128)],
        supplied_csrf: Annotated[
            str, Form(alias="csrf_token", min_length=1, max_length=128)
        ],
        upload: Annotated[UploadFile, File()],
    ) -> HTMLResponse:
        _verify_same_origin(request, supplied_csrf, csrf_token)
        if csv_import_coordinator is None or demo_mode:
            raise HTTPException(status_code=409, detail="CSV imports are disabled")
        filename = upload.filename or ""
        if len(filename) > 255:
            raise HTTPException(status_code=422, detail="invalid CSV filename")
        content = await upload.read(MAX_CSV_BYTES + 1)
        await upload.close()
        if len(content) > MAX_CSV_BYTES:
            raise HTTPException(status_code=413, detail="CSV is too large")
        try:
            preview = await csv_import_coordinator.preview(
                content=content,
                filename=filename,
                account_id=account_id,
            )
        except CsvImportError as error:
            raise HTTPException(status_code=422, detail=str(error)) from error
        except CsvImportConflictError as error:
            raise HTTPException(status_code=409, detail=str(error)) from error
        except UpstreamError as error:
            raise HTTPException(
                status_code=503, detail="Firefly could not prepare the CSV preview"
            ) from error
        return TEMPLATES.TemplateResponse(
            request=request,
            name="csv_import_preview.html",
            context={"preview": preview, "csrf_token": csrf_token},
        )

    @router.post("/imports/csv/commit", include_in_schema=False)
    async def csv_commit(request: Request) -> RedirectResponse:
        values = await _verified_form(request, csrf_token)
        if csv_import_coordinator is None or demo_mode:
            raise HTTPException(status_code=409, detail="CSV imports are disabled")
        preview_id = _one(values, "preview_id", max_length=128)
        try:
            imported = await csv_import_coordinator.commit(preview_id)
        except CsvImportConflictError as error:
            raise HTTPException(status_code=409, detail=str(error)) from error
        except (FireflyCsvImportWriteError, UpstreamError) as error:
            raise HTTPException(
                status_code=503, detail="Firefly could not complete the CSV import"
            ) from error
        refreshed = False
        if csv_post_import_sync is not None:
            try:
                await csv_post_import_sync()
                refreshed = True
            except Exception:  # noqa: BLE001 - the completed import must stay successful
                refreshed = False
        return RedirectResponse(
            f"/dashboard?csv_imported={imported}&csv_refreshed={str(refreshed).lower()}",
            status_code=status.HTTP_303_SEE_OTHER,
        )

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
            proposal = state.get_pending(proposal_id)
            if proposal is None or proposal.proposed_category is None:
                raise HTTPException(
                    status_code=409, detail="proposal is no longer approvable"
                )
            token = state.preview_batch((proposal_id,))
            transaction_id = _firefly_transaction_id(proposal.source)
            if category_updater is not None:
                await _update_category(
                    category_updater,
                    transaction_id,
                    proposal.original_category,
                    proposal.proposed_category,
                )
            try:
                state.approve_batch((proposal_id,), preview_token=token)
            except Exception:
                if category_updater is not None:
                    await category_updater(
                        transaction_id,
                        proposal.proposed_category,
                        proposal.original_category,
                    )
                raise
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
            if category_updater is not None:
                transaction_id = _firefly_transaction_id(proposal.source)
                await _update_category(
                    category_updater,
                    transaction_id,
                    proposal.original_category,
                    category,
                )
            from finance_assistant.categorisation_service import source_identity
            from finance_assistant.domain.categorisation import CalibrationExample

            try:
                state.record_correction(
                    CalibrationExample(
                        source_identity=source_identity(proposal.source),
                        feature_hash=proposal.feature_hash,
                        normalized_merchant=proposal.normalized_merchant,
                        approved_category=category,
                    )
                )
                state.resolve_proposal(proposal_id, status=ProposalStatus.CORRECTED)
            except Exception:
                if category_updater is not None:
                    await category_updater(
                        transaction_id, category, proposal.original_category
                    )
                raise
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
    _verify_same_origin(request, supplied, expected_token)
    return values


def _verify_same_origin(request: Request, supplied: str, expected_token: str) -> None:
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


def _import_status(request: Request) -> str | None:
    raw = request.query_params.get("csv_imported")
    if raw is None:
        return None
    try:
        count = int(raw)
    except ValueError:
        return None
    if not 0 <= count <= 10_000:
        return None
    message = (
        f"Imported {count} new transaction{'s' if count != 1 else ''} into Firefly."
    )
    if request.query_params.get("csv_refreshed") == "true":
        return f"{message} The dashboard is refreshed."
    if request.query_params.get("csv_refreshed") == "false":
        return f"{message} Dashboard refresh will retry on the next sync."
    return message


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


async def _update_category(
    updater: CategoryUpdater,
    transaction_id: str,
    expected_category: str | None,
    category: str | None,
) -> None:
    try:
        await updater(transaction_id, expected_category, category)
    except FireflyCategoryConflictError as error:
        raise HTTPException(status_code=409, detail=str(error)) from error
    except FireflyCategoryWriteError as error:
        raise HTTPException(
            status_code=503, detail="Firefly could not save the reviewed category"
        ) from error


def _firefly_transaction_id(source: SourceReference) -> str:
    if (
        source.system is not SourceSystem.FIREFLY_III
        or source.entity is not SourceEntity.TRANSACTION
    ):
        raise HTTPException(
            status_code=409,
            detail="only Firefly transaction categories can be updated",
        )
    return source.external_id
