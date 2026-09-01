"""Validated models used by the deterministic domain layer."""

from datetime import date
from decimal import Decimal
from enum import StrEnum

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator


class StrictModel(BaseModel):
    """Base model that rejects unknown input fields."""

    model_config = ConfigDict(extra="forbid", frozen=True)


class Currency(StrictModel):
    """Currency identity and the supported number of cash decimal places."""

    code: str = Field(min_length=3, max_length=3)
    minor_unit: int = Field(ge=0, le=18)

    @field_validator("code")
    @classmethod
    def normalize_code(cls, value: str) -> str:
        normalized = value.upper()
        if not normalized.isascii() or not normalized.isalpha():
            raise ValueError("currency code must contain exactly three ASCII letters")
        return normalized


class Money(StrictModel):
    """An exact cash amount paired with explicit currency precision."""

    amount: Decimal = Field(max_digits=38)
    currency: Currency

    @field_validator("amount", mode="before")
    @classmethod
    def reject_binary_float(cls, value: object) -> object:
        if isinstance(value, (bool, float)):
            raise ValueError("amount must be supplied as a decimal string or integer")
        return value

    @field_validator("amount")
    @classmethod
    def require_finite_amount(cls, value: Decimal) -> Decimal:
        if not value.is_finite():
            raise ValueError("amount must be finite")
        return value

    @model_validator(mode="after")
    def require_supported_precision(self) -> Money:
        digits = list(self.amount.as_tuple().digits)
        exponent = self.amount.as_tuple().exponent
        if not isinstance(exponent, int):  # pragma: no cover - finite values use int
            raise ValueError("amount exponent must be finite")
        while digits and digits[-1] == 0:
            digits.pop()
            exponent += 1
        fractional_places = max(-exponent, 0)
        if fractional_places > self.currency.minor_unit:
            raise ValueError(
                "amount precision exceeds the currency minor unit; "
                "rounding must be explicit"
            )
        return self


class SavingsGoal(StrictModel):
    """Savings target and current progress."""

    goal_id: str = Field(min_length=1, max_length=128)
    name: str = Field(min_length=1, max_length=256)
    target: Money
    current: Money
    started_on: date
    target_date: date

    @model_validator(mode="after")
    def validate_goal(self) -> SavingsGoal:
        if self.current.currency != self.target.currency:
            raise ValueError("target and current amounts must use the same currency")
        if self.target.amount <= 0:
            raise ValueError("target amount must be positive")
        if self.current.amount < 0:
            raise ValueError("current amount cannot be negative")
        if self.target_date <= self.started_on:
            raise ValueError("target_date must be later than started_on")
        return self


class GoalEvaluation(StrictModel):
    """Deterministic savings progress result."""

    goal_id: str
    currency: Currency
    completed: bool
    overdue: bool
    on_track: bool
    remaining: Decimal
    expected_saved: Decimal
    schedule_variance: Decimal
    required_weekly_contribution: Decimal
    days_remaining: int


class AlertSeverity(StrEnum):
    """Ordered human-facing alert level."""

    NONE = "none"
    HEADS_UP = "heads_up"
    WARNING = "warning"
    CRITICAL = "critical"


class BudgetWindow(StrictModel):
    """Inputs for one category and one budget period."""

    category: str = Field(min_length=1, max_length=128)
    budget: Money
    spent: Money
    available_cash: Money
    upcoming_committed: Money
    planned_savings: Money
    minimum_buffer: Money
    elapsed_days: int = Field(ge=0)
    total_days: int = Field(gt=0)

    @model_validator(mode="after")
    def validate_window(self) -> BudgetWindow:
        amounts = (
            self.budget,
            self.spent,
            self.available_cash,
            self.upcoming_committed,
            self.planned_savings,
            self.minimum_buffer,
        )
        currencies = {money.currency for money in amounts}
        if len(currencies) != 1:
            raise ValueError("all budget amounts must use the same currency")
        if any(money.amount < 0 for money in amounts):
            raise ValueError("budget amounts cannot be negative")
        if self.budget.amount == 0:
            raise ValueError("budget amount must be positive")
        if self.elapsed_days > self.total_days:
            raise ValueError("elapsed_days cannot exceed total_days")
        return self


class BudgetEvaluation(StrictModel):
    """Explainable output from the spending alert engine."""

    category: str
    currency: Currency
    severity: AlertSeverity
    spent_ratio: Decimal
    elapsed_ratio: Decimal
    pace_ratio: Decimal
    forecast_spend: Decimal
    remaining_budget: Decimal
    safe_to_spend: Decimal
    reasons: tuple[str, ...]
