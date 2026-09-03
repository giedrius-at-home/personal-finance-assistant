"""Versioned, previewable owner settings for local alerts and notifications."""

from __future__ import annotations

import hashlib
import json
from decimal import Decimal
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import Field, model_validator

from finance_assistant.domain.models import StrictModel


class OwnerSettings(StrictModel):
    policy_version: str = "2026-09-v1"
    timezone: str = "Europe/Amsterdam"
    evaluation_hour: int = Field(default=18, ge=0, le=23)
    notifications_enabled: bool = False
    heads_up_spent_ratio: Decimal = Field(default=Decimal("0.65"), ge=0, le=1)
    heads_up_pace_ratio: Decimal = Field(default=Decimal("1.25"), gt=0, le=10)
    heads_up_cooldown_hours: int = Field(default=168, ge=1, le=744)
    warning_cooldown_hours: int = Field(default=72, ge=1, le=744)
    critical_cooldown_hours: int = Field(default=24, ge=1, le=744)

    @model_validator(mode="after")
    def validate_settings(self) -> OwnerSettings:
        try:
            ZoneInfo(self.timezone)
        except ZoneInfoNotFoundError as error:
            raise ValueError("timezone is unknown") from error
        if not (
            self.critical_cooldown_hours
            <= self.warning_cooldown_hours
            <= self.heads_up_cooldown_hours
        ):
            raise ValueError("cooldowns must shorten as severity increases")
        return self

    @property
    def content_hash(self) -> str:
        payload = json.dumps(
            self.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        )
        return hashlib.sha256(payload.encode()).hexdigest()


class SettingsPreview(StrictModel):
    token: str = Field(pattern=r"^[0-9a-f]{64}$")
    before: OwnerSettings
    after: OwnerSettings
