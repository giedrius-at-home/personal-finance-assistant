import json
from pathlib import Path

import pytest

from scripts.manage import (
    BACKUP_SCHEMA_VERSION,
    REQUIRED_BACKUP_FILES,
    OperationError,
    bootstrap,
    health_urls,
    load_env,
    render_secret_template,
    sha256_file,
    verify_backup,
    write_private,
)


def test_render_secret_template_generates_distinct_expected_lengths() -> None:
    values = iter(("a" * 32, "b" * 32, "c" * 64))

    rendered = render_secret_template(
        "ONE=__GENERATE_32_HEX__\nTWO=__GENERATE_32_HEX__\nTHREE=__GENERATE_64_HEX__\n",
        lambda _: next(values),
    )

    assert rendered == f"ONE={'a' * 32}\nTWO={'b' * 32}\nTHREE={'c' * 64}\n"


def test_render_secret_template_rejects_unknown_marker() -> None:
    with pytest.raises(OperationError, match="unknown secret marker"):
        render_secret_template("SECRET=__GENERATE_UNKNOWN__\n")


def test_bootstrap_creates_private_file_and_refuses_overwrite(tmp_path: Path) -> None:
    template = tmp_path / "example"
    output = tmp_path / ".env"
    template.write_text("SECRET=__GENERATE_32_HEX__\n", encoding="utf-8")

    assert bootstrap(template, output) == output
    assert output.stat().st_mode & 0o777 == 0o600
    assert len(output.read_text(encoding="utf-8").split("=", 1)[1].strip()) == 32
    with pytest.raises(OperationError, match="refusing to overwrite"):
        bootstrap(template, output)


def test_write_private_never_overwrites(tmp_path: Path) -> None:
    destination = tmp_path / "private"
    write_private(destination, b"first")

    with pytest.raises(FileExistsError):
        write_private(destination, b"second")

    assert destination.read_bytes() == b"first"


def test_load_env_and_health_urls(tmp_path: Path) -> None:
    environment_file = tmp_path / ".env"
    environment_file.write_text(
        "# comment\n"
        "PFA_GATEWAY_PORT=8000\n"
        "PFA_FIREFLY_PORT=8080\n"
        "PFA_IMPORTER_PORT=8081\n"
        "PFA_GHOSTFOLIO_PORT=3333\n",
        encoding="utf-8",
    )

    environment = load_env(environment_file)

    assert health_urls(environment)["ghostfolio"].endswith("3333/api/v1/health")
    assert health_urls(environment)["ollama"] == "http://127.0.0.1:11434/api/tags"


def test_load_env_rejects_invalid_line(tmp_path: Path) -> None:
    environment_file = tmp_path / ".env"
    environment_file.write_text("invalid\n", encoding="utf-8")

    with pytest.raises(OperationError, match="invalid environment line"):
        load_env(environment_file)


def test_verify_backup_detects_tampering(tmp_path: Path) -> None:
    for name in REQUIRED_BACKUP_FILES:
        (tmp_path / name).write_text(f"synthetic {name}", encoding="utf-8")
    checksums = {name: sha256_file(tmp_path / name) for name in REQUIRED_BACKUP_FILES}
    (tmp_path / "metadata.json").write_text(
        json.dumps({"schema_version": BACKUP_SCHEMA_VERSION, "checksums": checksums}),
        encoding="utf-8",
    )

    verify_backup(tmp_path)
    (tmp_path / REQUIRED_BACKUP_FILES[0]).write_text("tampered", encoding="utf-8")

    with pytest.raises(OperationError, match="backup checksum failed"):
        verify_backup(tmp_path)
