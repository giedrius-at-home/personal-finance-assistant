"""Bootstrap, inspect, back up, and restore the local Compose stack."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets

# Only fixed Docker commands are executed and the host shell is never enabled.
import subprocess  # nosec B404
import sys
import urllib.error
import urllib.request
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from pathlib import Path
from typing import BinaryIO

ROOT = Path(__file__).resolve().parents[1]
ENV_TEMPLATE = ROOT / ".env.example"
ENV_FILE = ROOT / ".env"
BACKUP_ROOT = ROOT / "backups"
BACKUP_SCHEMA_VERSION = 2

SECRET_MARKERS = {
    "__GENERATE_32_HEX__": 16,
    "__GENERATE_64_HEX__": 32,
}
REQUIRED_BACKUP_FILES = (
    "assistant.sqlite3",
    "firefly.sql",
    "firefly-upload.tar.gz",
    "ghostfolio.sql",
    "images.txt",
)


class OperationError(RuntimeError):
    """Raised when a local operation cannot complete safely."""


def render_secret_template(
    template: str, token_hex: Callable[[int], str] = secrets.token_hex
) -> str:
    """Replace every secret marker with a distinct cryptographic token."""
    rendered_lines: list[str] = []
    for line in template.splitlines(keepends=True):
        rendered = line
        for marker, byte_count in SECRET_MARKERS.items():
            if marker in rendered:
                rendered = rendered.replace(marker, token_hex(byte_count), 1)
        rendered_lines.append(rendered)
    result = "".join(rendered_lines)
    if "__GENERATE_" in result:
        raise OperationError("unknown secret marker remains in template")
    return result


def write_private(path: Path, data: str | bytes) -> None:
    """Create a new owner-only file without overwriting existing data."""
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(path, flags, 0o600)
    mode = "wb" if isinstance(data, bytes) else "w"
    try:
        with os.fdopen(descriptor, mode) as stream:
            stream.write(data)
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def bootstrap(template: Path = ENV_TEMPLATE, output: Path = ENV_FILE) -> Path:
    """Generate the untracked local environment once."""
    if output.exists():
        raise OperationError(f"refusing to overwrite existing {output}")
    rendered = render_secret_template(template.read_text(encoding="utf-8"))
    write_private(output, rendered)
    return output


def load_env(path: Path = ENV_FILE) -> dict[str, str]:
    """Read the simple key/value environment format used by this project."""
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise OperationError(f"invalid environment line: {raw_line!r}")
        values[key] = value
    return values


def compose_command(*arguments: str) -> list[str]:
    """Build one explicit Compose command using the generated environment."""
    return ["docker", "compose", "--env-file", str(ENV_FILE), *arguments]


def run(
    command: Sequence[str],
    *,
    stdin: BinaryIO | None = None,
    stdout: BinaryIO | int | None = None,
) -> subprocess.CompletedProcess[bytes]:
    """Run a checked command without a host shell."""
    try:
        # The executable and every operational subcommand are fixed by this module.
        return subprocess.run(  # noqa: S603  # nosec B603
            command,
            cwd=ROOT,
            check=True,
            stdin=stdin,
            stdout=stdout,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        raise OperationError(f"command not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode(errors="replace").strip()
        raise OperationError(
            f"command failed: {' '.join(command)}\n{detail}"
        ) from error


def capture_private(command: Sequence[str], destination: Path) -> None:
    """Stream command output into a newly created private file."""
    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            run(command, stdout=stream)
    except BaseException:
        destination.unlink(missing_ok=True)
        raise


def sha256_file(path: Path) -> str:
    """Return a streaming SHA-256 checksum."""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def backup(destination_root: Path = BACKUP_ROOT) -> Path:
    """Create a private, checksummed backup of both durable applications."""
    if not ENV_FILE.exists():
        raise OperationError("missing .env; run make bootstrap first")
    destination_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    destination_root.chmod(0o700)
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    partial = destination_root / f".{timestamp}.partial"
    final = destination_root / timestamp
    partial.mkdir(mode=0o700)

    try:
        quiesced_services = ("gateway", "importer", "firefly-cron", "ghostfolio")
        run(compose_command("stop", *quiesced_services))
        try:
            run(compose_command("exec", "-T", "firefly", "php", "artisan", "down"))
            try:
                capture_private(
                    compose_command(
                        "exec",
                        "-T",
                        "firefly-db",
                        "sh",
                        "-ec",
                        'MYSQL_PWD="$MYSQL_PASSWORD" exec mariadb-dump '
                        "--single-transaction --add-drop-table --user=firefly firefly",
                    ),
                    partial / "firefly.sql",
                )
                capture_private(
                    compose_command(
                        "exec",
                        "-T",
                        "firefly",
                        "tar",
                        "-C",
                        "/var/www/html/storage/upload",
                        "-czf",
                        "-",
                        ".",
                    ),
                    partial / "firefly-upload.tar.gz",
                )
                capture_private(
                    compose_command(
                        "exec",
                        "-T",
                        "ghostfolio-db",
                        "sh",
                        "-ec",
                        "exec pg_dump --clean --if-exists --no-owner --no-privileges "
                        '-U "$POSTGRES_USER" "$POSTGRES_DB"',
                    ),
                    partial / "ghostfolio.sql",
                )
                capture_private(
                    compose_command(
                        "run",
                        "--rm",
                        "--no-deps",
                        "-T",
                        "gateway",
                        "python",
                        "-c",
                        "import pathlib,sys;sys.stdout.buffer.write("
                        "pathlib.Path('/var/lib/pfa/assistant.sqlite3').read_bytes())",
                    ),
                    partial / "assistant.sqlite3",
                )
                capture_private(compose_command("images"), partial / "images.txt")

                checksums = {
                    name: sha256_file(partial / name) for name in REQUIRED_BACKUP_FILES
                }
                metadata = {
                    "schema_version": BACKUP_SCHEMA_VERSION,
                    "created_at": datetime.now(UTC).isoformat(),
                    "checksums": checksums,
                }
                write_private(
                    partial / "metadata.json",
                    json.dumps(metadata, indent=2, sort_keys=True) + "\n",
                )
                partial.rename(final)
            finally:
                run(compose_command("exec", "-T", "firefly", "php", "artisan", "up"))
        finally:
            run(compose_command("up", "--detach", *quiesced_services))
    except BaseException:
        if partial.exists():
            for child in partial.glob("*"):
                child.unlink(missing_ok=True)
            partial.rmdir()
        raise
    return final


def verify_backup(source: Path) -> None:
    """Validate backup structure and checksums before any destructive action."""
    metadata_path = source / "metadata.json"
    if not metadata_path.is_file():
        raise OperationError("backup metadata is missing")
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if metadata.get("schema_version") != BACKUP_SCHEMA_VERSION:
        raise OperationError("unsupported backup schema version")
    checksums = metadata.get("checksums")
    if not isinstance(checksums, dict):
        raise OperationError("backup checksums are missing")
    for name in REQUIRED_BACKUP_FILES:
        path = source / name
        if not path.is_file() or checksums.get(name) != sha256_file(path):
            raise OperationError(f"backup checksum failed: {name}")


def restore(source: Path, *, confirmed: bool) -> None:
    """Restore both application databases and Firefly uploads."""
    if not confirmed:
        raise OperationError("restore requires --confirm-restore")
    verify_backup(source)
    application_services = (
        "gateway",
        "importer",
        "firefly-cron",
        "firefly",
        "ghostfolio",
    )
    run(compose_command("stop", *application_services))
    try:
        with (source / "firefly.sql").open("rb") as stream:
            run(
                compose_command(
                    "exec",
                    "-T",
                    "firefly-db",
                    "sh",
                    "-ec",
                    'MYSQL_PWD="$MYSQL_PASSWORD" exec mariadb --user=firefly firefly',
                ),
                stdin=stream,
                stdout=subprocess.DEVNULL,
            )
        with (source / "ghostfolio.sql").open("rb") as stream:
            run(
                compose_command(
                    "exec",
                    "-T",
                    "ghostfolio-db",
                    "sh",
                    "-ec",
                    "exec psql --set ON_ERROR_STOP=on "
                    '-U "$POSTGRES_USER" "$POSTGRES_DB"',
                ),
                stdin=stream,
                stdout=subprocess.DEVNULL,
            )
        with (source / "firefly-upload.tar.gz").open("rb") as stream:
            run(
                compose_command(
                    "run",
                    "--rm",
                    "--no-deps",
                    "-T",
                    "firefly",
                    "tar",
                    "-C",
                    "/var/www/html/storage/upload",
                    "-xzf",
                    "-",
                ),
                stdin=stream,
                stdout=subprocess.DEVNULL,
            )
        with (source / "assistant.sqlite3").open("rb") as stream:
            run(
                compose_command(
                    "run",
                    "--rm",
                    "--no-deps",
                    "-T",
                    "gateway",
                    "python",
                    "-c",
                    "import pathlib,sys;pathlib.Path("
                    "'/var/lib/pfa/assistant.sqlite3').write_bytes("
                    "sys.stdin.buffer.read())",
                ),
                stdin=stream,
                stdout=subprocess.DEVNULL,
            )
    finally:
        run(compose_command("up", "--detach", *application_services))


def health_urls(environment: dict[str, str]) -> dict[str, str]:
    """Build local health URLs from the generated configuration."""
    return {
        "gateway": f"http://127.0.0.1:{environment['PFA_GATEWAY_PORT']}/health",
        "firefly": f"http://127.0.0.1:{environment['PFA_FIREFLY_PORT']}/health",
        "importer": f"http://127.0.0.1:{environment['PFA_IMPORTER_PORT']}/",
        "ghostfolio": (
            f"http://127.0.0.1:{environment['PFA_GHOSTFOLIO_PORT']}/api/v1/health"
        ),
        "ollama": "http://127.0.0.1:11434/api/tags",
    }


def doctor() -> bool:
    """Check every host-visible service without printing response bodies."""
    environment = load_env()
    healthy = True
    for name, url in health_urls(environment).items():
        if not url.startswith("http://127.0.0.1:"):
            raise OperationError(f"doctor refused non-loopback URL for {name}")
        try:
            # URLs are constructed locally above and constrained to HTTP loopback.
            with urllib.request.urlopen(url, timeout=5) as response:  # noqa: S310  # nosec B310
                status = response.status
            result = "ok" if 200 <= status < 400 else f"HTTP {status}"
        except (OSError, urllib.error.URLError) as error:
            if name == "ollama":
                result = f"optional, unavailable ({error})"
            else:
                healthy = False
                result = f"unavailable ({error})"
        print(f"{name}: {result}")
    return healthy


def build_parser() -> argparse.ArgumentParser:
    """Build the management command parser."""
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("bootstrap", help="generate .env without overwriting")
    subparsers.add_parser("doctor", help="check host-visible service health")
    subparsers.add_parser("backup", help="create a checksummed local backup")
    restore_parser = subparsers.add_parser("restore", help="restore a local backup")
    restore_parser.add_argument("backup", type=Path)
    restore_parser.add_argument("--confirm-restore", action="store_true")
    return parser


def main(arguments: Sequence[str] | None = None) -> int:
    """Run one management operation."""
    parsed = build_parser().parse_args(arguments)
    try:
        if parsed.command == "bootstrap":
            print(f"created {bootstrap()}")
        elif parsed.command == "doctor":
            return 0 if doctor() else 1
        elif parsed.command == "backup":
            print(f"created {backup()}")
        elif parsed.command == "restore":
            restore(parsed.backup.resolve(), confirmed=parsed.confirm_restore)
            print(f"restored {parsed.backup}")
    except (OperationError, OSError, json.JSONDecodeError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
