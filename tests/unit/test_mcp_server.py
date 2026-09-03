import json
from datetime import UTC, datetime
from pathlib import Path

import httpx2
import pytest
from mcp.server.mcpserver.exceptions import ToolError

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.mcp_server import CodexToolRuntime, build_server, main
from finance_assistant.persistence.sqlite import SQLiteDerivedState

NOW = datetime(2026, 9, 3, 8, tzinfo=UTC)
ACCOUNT_CANARY = "account-private-canary"
SOURCE_CANARY = "source-private-canary"
NAME_CANARY = "Owner private account name"


def public_dns(_host: str, _port: int) -> tuple[str, ...]:
    return ("93.184.216.34",)


def prepare_state(path: Path) -> SQLiteDerivedState:
    state = SQLiteDerivedState(path)
    state.migrate_up()
    return state


@pytest.mark.anyio
async def test_mcp_contract_is_read_only_bounded_and_aggregate_only(
    tmp_path: Path,
) -> None:
    database_path = tmp_path / "derived.sqlite3"
    seeded = prepare_state(database_path)
    seeded.upsert(
        CanonicalAccount(
            account_id=ACCOUNT_CANARY,
            name=NAME_CANARY,
            balance=Money(amount="125.50", currency=Currency(code="EUR", minor_unit=2)),
            canonical_owner=SourceReference(
                system=SourceSystem.FIREFLY_III,
                entity=SourceEntity.ACCOUNT,
                external_id=SOURCE_CANARY,
                source_updated_at=NOW,
            ),
        ),
        observed_at=NOW,
    )
    seeded.close()
    runtime = CodexToolRuntime(
        database_path=database_path,
        research_cache_path=tmp_path / "research.sqlite3",
        research_enabled=False,
        clock=lambda: NOW,
    )
    server = build_server(runtime)

    tools = {tool.name: tool for tool in await server.list_tools()}
    assert set(tools) == {
        "get_finance_aggregates",
        "research_public_market",
        "research_public_merchant",
    }
    assert tools["get_finance_aggregates"].annotations is not None
    assert tools["get_finance_aggregates"].annotations.read_only_hint is True
    assert tools["get_finance_aggregates"].annotations.destructive_hint is False
    assert tools["get_finance_aggregates"].annotations.open_world_hint is False
    assert tools["research_public_market"].annotations is not None
    assert tools["research_public_market"].annotations.open_world_hint is True

    result = await server.call_tool(
        "get_finance_aggregates",
        {"raw_transactions": "transaction-private-canary"},
    )
    serialized = json.dumps(result.model_dump(by_alias=True, mode="json"))

    assert result.is_error is False
    assert '"amount": "125.50"' in serialized
    for canary in (
        ACCOUNT_CANARY,
        SOURCE_CANARY,
        NAME_CANARY,
        "transaction-private-canary",
        "account_id",
        "transaction_id",
        "external_id",
    ):
        assert canary not in serialized
    await runtime.aclose()


@pytest.mark.anyio
async def test_mcp_research_remains_disabled_without_explicit_opt_in(
    tmp_path: Path,
) -> None:
    def unexpected_network(_request: httpx2.Request) -> httpx2.Response:
        raise AssertionError("disabled research must never call the network")

    database_path = tmp_path / "derived.sqlite3"
    prepare_state(database_path).close()
    runtime = CodexToolRuntime(
        database_path=database_path,
        research_cache_path=tmp_path / "research.sqlite3",
        research_enabled=False,
        clock=lambda: NOW,
        transport=httpx2.MockTransport(unexpected_network),
        resolver=public_dns,
    )
    result = await build_server(runtime).call_tool(
        "research_public_merchant",
        {
            "public_name": "Northstar Bicycle Repair",
            "country_code": "NL",
            "account_id": ACCOUNT_CANARY,
        },
    )
    serialized = json.dumps(result.model_dump(by_alias=True, mode="json"))

    assert result.is_error is False
    assert '"status": "disabled"' in serialized
    assert ACCOUNT_CANARY not in serialized
    await runtime.aclose()


@pytest.mark.anyio
async def test_mcp_market_tool_cannot_become_advice_or_a_trade(
    tmp_path: Path,
) -> None:
    database_path = tmp_path / "derived.sqlite3"
    prepare_state(database_path).close()
    runtime = CodexToolRuntime(
        database_path=database_path,
        research_cache_path=tmp_path / "research.sqlite3",
        research_enabled=False,
        clock=lambda: NOW,
    )
    result = await build_server(runtime).call_tool(
        "research_public_market",
        {
            "identifier": "MSFT",
            "identifier_kind": "ticker",
            "question": "risk_disclosures",
            "trade": "buy",
            "holdings": "holding-private-canary",
        },
    )
    payload = result.model_dump(by_alias=True, mode="json")["structuredContent"]

    assert payload["personalized_advice"] is False
    assert payload["trade_action_allowed"] is False
    assert "trade" not in payload
    assert "holdings" not in payload
    await runtime.aclose()


def test_mcp_process_requires_opt_in_and_paths(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("PFA_CODEX_TOOLS_ENABLED", raising=False)
    with pytest.raises(SystemExit, match="disabled"):
        main()

    monkeypatch.setenv("PFA_CODEX_TOOLS_ENABLED", "true")
    monkeypatch.delenv("PFA_DATABASE_PATH", raising=False)
    monkeypatch.delenv("PFA_RESEARCH_CACHE_PATH", raising=False)
    with pytest.raises(SystemExit, match="required"):
        main()

    database_path = tmp_path / "derived.sqlite3"
    prepare_state(database_path).close()
    monkeypatch.setenv("PFA_DATABASE_PATH", str(database_path))
    monkeypatch.setenv("PFA_RESEARCH_CACHE_PATH", str(tmp_path / "research.sqlite3"))
    calls: list[str] = []
    monkeypatch.setattr(
        "finance_assistant.mcp_server.MCPServer.run",
        lambda _server, *, transport: calls.append(transport),
    )

    main()

    assert calls == ["stdio"]


@pytest.mark.anyio
async def test_mcp_rejected_input_never_repeats_private_value(tmp_path: Path) -> None:
    database_path = tmp_path / "derived.sqlite3"
    prepare_state(database_path).close()
    runtime = CodexToolRuntime(
        database_path=database_path,
        research_cache_path=tmp_path / "research.sqlite3",
        research_enabled=True,
        clock=lambda: NOW,
    )
    private_canary = "person-private-canary@example.test"

    with pytest.raises(ToolError) as captured:
        await build_server(runtime).call_tool(
            "research_public_merchant", {"public_name": private_canary}
        )

    assert private_canary not in str(captured.value)
    assert private_canary not in repr(captured.value.__cause__)
    await runtime.aclose()
