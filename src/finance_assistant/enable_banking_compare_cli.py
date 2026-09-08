"""Print privacy-safe transaction coverage from a direct Enable Banking session."""

from __future__ import annotations

import argparse
import asyncio
import os
from datetime import date
from pathlib import Path

from pydantic import SecretStr

from finance_assistant.integrations.enable_banking import EnableBankingClient

MAX_SECRET_BYTES = 4096


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--from", dest="date_from", type=date.fromisoformat, required=True
    )
    result.add_argument("--to", dest="date_to", type=date.fromisoformat, required=True)
    return result


def _read_secret(path: Path) -> SecretStr:
    if not path.is_file():
        raise ValueError("session ID file is not configured")
    if path.stat().st_size > MAX_SECRET_BYTES:
        raise ValueError("session ID file exceeds the size limit")
    value = path.read_text(encoding="utf-8").strip()
    if not value:
        raise ValueError("session ID file is empty")
    return SecretStr(value)


async def _run(date_from: date, date_to: date) -> None:
    app_id = SecretStr(os.environ.get("PFA_ENABLE_BANKING_APP_ID", ""))
    key_path = Path(
        os.environ.get(
            "PFA_ENABLE_BANKING_PRIVATE_KEY_FILE",
            "/run/secrets/enable-banking-private-key.pem",
        )
    )
    session_id = _read_secret(
        Path(
            os.environ.get(
                "PFA_ENABLE_BANKING_SESSION_ID_FILE",
                "/run/secrets/enable-banking-session-id.txt",
            )
        )
    )
    async with EnableBankingClient(app_id=app_id, private_key_path=key_path) as client:
        snapshot = await client.coverage_snapshot(
            session_id=session_id, date_from=date_from, date_to=date_to
        )
    print(f"accounts={len(snapshot.accounts)}")
    print(f"session_valid_until={snapshot.session_valid_until or 'unknown'}")
    for index, account in enumerate(snapshot.accounts, start=1):
        print(
            f"account={index} transactions={account.transaction_count} "
            f"pages={account.page_count} earliest={account.earliest_date or 'unknown'} "
            f"latest={account.latest_date or 'unknown'}"
        )


def main() -> None:
    arguments = parser().parse_args()
    asyncio.run(_run(arguments.date_from, arguments.date_to))


if __name__ == "__main__":
    main()
