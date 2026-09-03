import plistlib
from pathlib import Path

import pytest

from scripts.macos_app import (
    APP_NAME,
    BUNDLE_IDENTIFIER,
    LauncherError,
    install,
    launcher_script,
)


def fake_icon(_source: Path, destination: Path) -> None:
    destination.write_bytes(b"synthetic icns")


def test_launcher_starts_only_local_stack_and_opens_dashboard() -> None:
    script = launcher_script()

    assert "docker start $containers" in script
    assert "http://127.0.0.1:8000/dashboard" in script
    assert "colima start" in script
    assert ".env" not in script
    assert "compose" not in script
    assert "PFA_CODEX_TOOLS_ENABLED" not in script
    assert "PFA_RESEARCH_ENABLED" not in script


def test_install_creates_owner_app_bundle_without_embedding_env(tmp_path: Path) -> None:
    destination = tmp_path / APP_NAME

    installed = install(
        destination,
        icon_builder=fake_icon,
        platform="darwin",
    )

    executable = installed / "Contents" / "MacOS" / "launch"
    metadata = plistlib.loads((installed / "Contents" / "Info.plist").read_bytes())
    assert executable.stat().st_mode & 0o777 == 0o755
    assert metadata["CFBundleIdentifier"] == BUNDLE_IDENTIFIER
    assert (installed / "Contents" / "Resources" / "AppIcon.icns").is_file()
    assert "SYNTHETIC=only" not in executable.read_text(encoding="utf-8")

    with pytest.raises(LauncherError, match="refusing to overwrite"):
        install(
            destination,
            icon_builder=fake_icon,
            platform="darwin",
        )


def test_install_rejects_non_macos(tmp_path: Path) -> None:
    with pytest.raises(LauncherError, match="only be installed on macOS"):
        install(tmp_path / APP_NAME, platform="linux")
