"""Fail-closed evidence collection; no bank connection or personal fixtures."""

import asyncio
import json
import stat
from datetime import UTC, datetime

import pytest

from scripts.trade_republic_capture import CaptureError, pages, private_json, recent


class Feed:
    def __init__(self, payloads):
        self.payloads = iter(payloads)
        self.unsubscribed = []

    async def subscribe(self, request):
        return "1"

    async def recv(self):
        return "1", {}, next(self.payloads)

    async def unsubscribe(self, subscription):
        self.unsubscribed.append(subscription)


def page(items, after=None):
    return {"items": items, "cursors": {"after": after}}


def test_exhaustive_pages_and_identical_duplicate():
    feed = Feed([page([{"id": "a"}], "cursor"), page([{"id": "a"}, {"id": "b"}])])
    assert asyncio.run(pages(feed, "timelineTransactions")) == [
        {"id": "a"},
        {"id": "b"},
    ]
    assert feed.unsubscribed == ["1", "1"]


@pytest.mark.parametrize(
    "payloads",
    [
        [page([{"id": "a"}], "loop"), page([{"id": "b"}], "loop")],
        [page([{"id": "a"}], "c1"), page([{"id": "a"}], "c2")],
        [page([{"id": "a"}], "c1"), page([{"id": "a", "changed": True}])],
        [{"items": []}],
        [page([{"id": ""}])],
        [page([], "cursor")],
        [page([{"id": str(i)}], str(i)) for i in range(200)],
    ],
)
def test_reject_incomplete_or_conflicting_feed(payloads):
    with pytest.raises(CaptureError):
        asyncio.run(pages(Feed(payloads), "timelineTransactions"))


def test_timezone_is_preserved_at_cutoff():
    event = {"timestamp": "2026-09-13T01:00:00+02:00"}
    assert recent([event], datetime(2026, 9, 13, tzinfo=UTC)) == []
    with pytest.raises(CaptureError, match="missing_timezone"):
        recent(
            [{"timestamp": "2026-09-13T01:00:00"}], datetime(2026, 9, 13, tzinfo=UTC)
        )


def test_evidence_is_private_and_cannot_overwrite(tmp_path):
    path = tmp_path / "evidence.json"
    private_json(path, {"first": True})
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    with pytest.raises(FileExistsError):
        private_json(path, {"first": False})
    assert json.loads(path.read_text()) == {"first": True}


def test_capture_mode_never_requests_credentials(monkeypatch, tmp_path, capsys):
    import importlib.metadata
    import sys
    import types

    import scripts.trade_republic_capture as module

    class Session:
        def __init__(self, **kwargs):
            self._websession = types.SimpleNamespace(request=lambda *a, **k: None)
            assert kwargs["phone_no"] == "session-only"
            assert kwargs["pin"] == "unused"
            assert kwargs["cookies_file"] == tmp_path / "cookies.txt"

        def resume_websession(self):
            return False

    monkeypatch.setattr(module, "PRIVATE", tmp_path)
    monkeypatch.setattr(sys, "argv", ["capture", "capture"])
    monkeypatch.setattr(importlib.metadata, "version", lambda _: "0.4.10")
    monkeypatch.setitem(
        sys.modules, "pytr.api", types.SimpleNamespace(TradeRepublicApi=Session)
    )
    with pytest.raises(SystemExit):
        module.main()
    assert json.loads(capsys.readouterr().out)["status"] == "reauthorization_required"
    assert not (tmp_path / "credentials").exists()


def test_provider_exception_never_leaks(monkeypatch, tmp_path, capsys):
    import importlib.metadata
    import sys
    import types

    import scripts.trade_republic_capture as module

    class Session:
        def __init__(self, **kwargs):
            raise RuntimeError("SENSITIVE_PROVIDER_PAYLOAD")

    monkeypatch.setattr(module, "PRIVATE", tmp_path)
    monkeypatch.setattr(sys, "argv", ["capture", "capture"])
    monkeypatch.setattr(importlib.metadata, "version", lambda _: "0.4.10")
    monkeypatch.setitem(
        sys.modules, "pytr.api", types.SimpleNamespace(TradeRepublicApi=Session)
    )
    with pytest.raises(SystemExit):
        module.main()
    output = capsys.readouterr()
    assert "SENSITIVE" not in output.out + output.err
    assert json.loads(output.out)["status"] == "capture_failed"


def test_capture_closes_socket_when_collection_fails(monkeypatch):
    import scripts.trade_republic_capture as module

    class Connection:
        closed = False

        async def close(self):
            self.closed = True

    async def fail(*args, **kwargs):
        raise CaptureError("invalid_page")

    connection = Connection()
    monkeypatch.setattr(module, "collect_capture", fail)
    with pytest.raises(CaptureError, match="invalid_page"):
        asyncio.run(module.capture(connection))
    assert connection.closed


def test_http_requests_have_bounded_default_timeout():
    import types

    from scripts.trade_republic_capture import bound_http_requests

    received = []
    tr = types.SimpleNamespace(
        _websession=types.SimpleNamespace(
            request=lambda *args, **kwargs: received.append(kwargs)
        )
    )
    bound_http_requests(tr)
    tr._websession.request("GET", "synthetic")
    assert received == [{"timeout": (10, 30)}]
