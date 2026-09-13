"""Run synthetic PHP adapter tests in the pinned importer image, without network."""

import re
import shutil
import subprocess  # nosec B404 - fixed test command with synthetic input
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    docker = shutil.which("docker")
    if docker is None:
        raise SystemExit("Docker is required for the PHP adapter tests")
    match = re.search(
        r"image: (fireflyiii/data-importer:\S+)", (ROOT / "compose.yaml").read_text()
    )
    if match is None:
        raise SystemExit("Pinned importer image is missing")
    adapter = (
        (ROOT / "scripts/bank_import_worker.php").read_text().removeprefix("<?php")
    )
    adapter = adapter.replace(
        "declare(strict_types=1);",
        "declare(strict_types=1); define('PFA_BANK_WORKER_TEST', true);",
    )
    tests = (
        (ROOT / "tests/php/bank_import_worker.php").read_text().removeprefix("<?php")
    )
    result = subprocess.run(  # noqa: S603  # nosec B603
        [
            docker,
            "run",
            "--rm",
            "--network",
            "none",
            "--entrypoint",
            "php",
            match[1],
            "-r",
            adapter + tests,
        ],
        check=False,
        timeout=120,
    )
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
