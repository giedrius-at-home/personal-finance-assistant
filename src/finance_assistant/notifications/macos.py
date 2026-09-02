"""Opt-in, argument-safe macOS notification delivery."""

from __future__ import annotations

import platform
import subprocess  # nosec B404
from collections.abc import Callable
from enum import StrEnum

from pydantic import Field

from finance_assistant.domain.models import StrictModel


class NotificationOutcome(StrEnum):
    DELIVERED = "delivered"
    DISABLED = "disabled"
    UNAVAILABLE = "unavailable"
    FAILED = "failed"


class NotificationMessage(StrictModel):
    title: str = Field(min_length=1, max_length=96)
    body: str = Field(min_length=1, max_length=512)


def _run(command: list[str]) -> None:
    subprocess.run(  # noqa: S603  # nosec B603
        command,
        check=True,
        timeout=10,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


class MacOSNotifier:
    """Send a local notification without interpolating data into AppleScript."""

    def __init__(
        self,
        *,
        enabled: bool = False,
        system: str | None = None,
        runner: Callable[[list[str]], None] = _run,
    ) -> None:
        self._enabled = enabled
        self._system = system or platform.system()
        self._runner = runner

    def send(self, message: NotificationMessage) -> NotificationOutcome:
        if not self._enabled:
            return NotificationOutcome.DISABLED
        if self._system != "Darwin":
            return NotificationOutcome.UNAVAILABLE
        command = [
            "/usr/bin/osascript",
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            "--",
            message.title,
            message.body,
        ]
        try:
            self._runner(command)
        except OSError, subprocess.SubprocessError:
            return NotificationOutcome.FAILED
        return NotificationOutcome.DELIVERED
