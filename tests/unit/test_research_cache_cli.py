from pathlib import Path

import pytest

from finance_assistant.research_cache import ResearchCache
from finance_assistant.research_cache_cli import main


def test_cache_purge_requires_confirmation_and_explicit_path(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("PFA_RESEARCH_CACHE_PATH", raising=False)
    with pytest.raises(SystemExit):
        main(())
    with pytest.raises(SystemExit):
        main(("--confirm-purge",))


def test_cache_purge_is_content_free(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    path = tmp_path / "research.sqlite3"
    cache = ResearchCache(path)
    cache.close()
    monkeypatch.setenv("PFA_RESEARCH_CACHE_PATH", str(path))

    assert main(("--confirm-purge",)) == 0
    assert capsys.readouterr().out == "purged 0 cached public research result(s)\n"
