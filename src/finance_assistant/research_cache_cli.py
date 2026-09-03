"""Explicit recovery command for the disposable public-research cache."""

from __future__ import annotations

import argparse
import os
from collections.abc import Sequence

from finance_assistant.research_cache import ResearchCache


def main(arguments: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--confirm-purge", action="store_true")
    parsed = parser.parse_args(arguments)
    if not parsed.confirm_purge:
        parser.error("purge requires --confirm-purge")
    path = os.getenv("PFA_RESEARCH_CACHE_PATH")
    if not path:
        parser.error("PFA_RESEARCH_CACHE_PATH is required")
    cache = ResearchCache(path)
    try:
        count = cache.purge()
    finally:
        cache.close()
    print(f"purged {count} cached public research result(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
