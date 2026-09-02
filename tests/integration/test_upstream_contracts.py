"""Opt-in contract checks against the pinned local upstream services."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

import pytest

from finance_assistant.integrations.firefly import FireflyClient
from finance_assistant.integrations.ghostfolio import GhostfolioClient
from scripts.manage import load_env

pytestmark = pytest.mark.upstream_contract

ROOT = Path(__file__).resolve().parents[2]


def local_environment(required_key: str) -> dict[str, str]:
    """Load ignored local credentials or skip ordinary offline test runs."""

    env_file = ROOT / ".env"
    required = os.environ.get("PFA_REQUIRE_UPSTREAM_CONTRACTS") == "1"
    if not env_file.is_file():
        if required:
            pytest.fail("live contracts require .env; run make bootstrap first")
        pytest.skip("local upstream configuration is unavailable")
    environment = load_env(env_file)
    if not environment.get(required_key):
        message = "live contracts require configured synthetic API credentials"
        if required:
            pytest.fail(message)
        pytest.skip(message)
    return environment


def test_firefly_contract_against_pinned_local_service() -> None:
    environment = local_environment("PFA_FIREFLY_API_TOKEN")

    async def verify() -> None:
        async with FireflyClient(
            base_url=f"http://127.0.0.1:{environment['PFA_FIREFLY_PORT']}/api/v1",
            token=environment["PFA_FIREFLY_API_TOKEN"],
        ) as client:
            currencies = await client.list_currencies()
            accounts = await client.list_accounts()
            transactions = await client.list_transactions()
        assert currencies
        assert isinstance(accounts, tuple)
        assert isinstance(transactions, tuple)

    asyncio.run(verify())


def test_ghostfolio_contract_against_pinned_local_service() -> None:
    environment = local_environment("PFA_GHOSTFOLIO_BEARER_TOKEN")

    async def verify() -> None:
        async with GhostfolioClient(
            base_url=f"http://127.0.0.1:{environment['PFA_GHOSTFOLIO_PORT']}/api/v1",
            bearer_token=environment["PFA_GHOSTFOLIO_BEARER_TOKEN"],
        ) as client:
            accounts = await client.list_accounts()
            activities = await client.list_activities()
        assert isinstance(accounts, tuple)
        assert isinstance(activities, tuple)

    asyncio.run(verify())
