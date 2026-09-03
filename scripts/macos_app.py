"""Install the local dashboard launcher as an owner-scoped macOS app."""

from __future__ import annotations

import argparse
import os
import plistlib
import subprocess  # nosec B404
import sys
import tempfile
from collections.abc import Callable, Sequence
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_NAME = "Personal Finance Assistant.app"
DEFAULT_DESTINATION = Path.home() / "Applications" / APP_NAME
ICON_SOURCE = ROOT / "assets" / "macos" / "AppIcon.svg"
BUNDLE_IDENTIFIER = "io.github.giedrius-at-home.personal-finance-assistant"

IconBuilder = Callable[[Path, Path], None]


class LauncherError(RuntimeError):
    """A bounded launcher installation failure."""


def launcher_script() -> str:
    """Render a no-terminal launcher for an existing local Compose stack."""

    return """#!/bin/zsh
set -u
umask 077
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
readonly dashboard_url="http://127.0.0.1:8000/dashboard"
readonly log_directory="$HOME/Library/Logs/Personal Finance Assistant"
readonly log_file="$log_directory/launcher.log"

mkdir -p "$log_directory"
chmod 700 "$log_directory"
if [[ -f "$log_file" ]] && \
    (( $(/usr/bin/stat -f %z "$log_file") > 1048576 )); then
  /bin/mv "$log_file" "$log_directory/launcher.previous.log"
fi
exec >>"$log_file" 2>&1
echo "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ) launch requested"

alert_failure() {{
  /usr/bin/osascript \
    -e 'display alert "PFA" message "Start failed. See logs." as critical' \
    >/dev/null
  exit 1
}}

if ! docker info >/dev/null 2>&1; then
  command -v colima >/dev/null 2>&1 || alert_failure
  colima start --cpu 4 --memory 8 --disk 60 \
    --vm-type vz --runtime docker || alert_failure
fi

readonly containers=(
  personal-finance-assistant-firefly-db-1
  personal-finance-assistant-ghostfolio-db-1
  personal-finance-assistant-ghostfolio-redis-1
  personal-finance-assistant-firefly-1
  personal-finance-assistant-ghostfolio-1
  personal-finance-assistant-importer-1
  personal-finance-assistant-firefly-cron-1
  personal-finance-assistant-gateway-1
)

for container in $containers; do
  if ! docker container inspect "$container" >/dev/null 2>&1; then
    echo "missing expected container: $container; run make up from the repository"
    alert_failure
  fi
done
docker start $containers >/dev/null || alert_failure
echo "expected local containers started"

for attempt in {{1..60}}; do
  if /usr/bin/curl --fail --silent --show-error \
      --max-time 2 http://127.0.0.1:8000/health >/dev/null; then
    /usr/bin/open "$dashboard_url" || alert_failure
    echo "dashboard opened"
    exit 0
  fi
  sleep 1
done

alert_failure
"""


def info_plist() -> dict[str, object]:
    return {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "Personal Finance Assistant",
        "CFBundleExecutable": "launch",
        "CFBundleIconFile": "AppIcon",
        "CFBundleIdentifier": BUNDLE_IDENTIFIER,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Personal Finance Assistant",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "0.1",
        "CFBundleVersion": "1",
        "LSMinimumSystemVersion": "13.0",
        "LSUIElement": True,
        "NSHighResolutionCapable": True,
    }


def _run(command: Sequence[str]) -> None:
    try:
        subprocess.run(  # noqa: S603  # nosec B603
            command,
            check=True,
            capture_output=True,
        )
    except FileNotFoundError as error:
        raise LauncherError(
            f"required macOS tool is unavailable: {command[0]}"
        ) from error
    except subprocess.CalledProcessError as error:
        raise LauncherError(f"macOS icon generation failed: {command[0]}") from error


def build_icon(source: Path, destination: Path) -> None:
    """Convert the reviewed SVG into a native multi-resolution ICNS file."""

    dimensions = (16, 32, 128, 256, 512)
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        base = work / "base.png"
        iconset = work / "AppIcon.iconset"
        iconset.mkdir()
        _run(("/usr/bin/sips", "-s", "format", "png", str(source), "--out", str(base)))
        for dimension in dimensions:
            for scale in (1, 2):
                pixels = dimension * scale
                suffix = "@2x" if scale == 2 else ""
                output = iconset / f"icon_{dimension}x{dimension}{suffix}.png"
                _run(
                    (
                        "/usr/bin/sips",
                        "-z",
                        str(pixels),
                        str(pixels),
                        str(base),
                        "--out",
                        str(output),
                    )
                )
        _run(
            (
                "/usr/bin/iconutil",
                "--convert",
                "icns",
                str(iconset),
                "--output",
                str(destination),
            )
        )


def install(
    destination: Path = DEFAULT_DESTINATION,
    *,
    icon_builder: IconBuilder = build_icon,
    platform: str = sys.platform,
) -> Path:
    """Create a new app bundle without overwriting an existing application."""

    if platform != "darwin":
        raise LauncherError("the dashboard app can only be installed on macOS")
    if destination.exists():
        raise LauncherError(f"refusing to overwrite existing {destination}")
    if not ICON_SOURCE.is_file():
        raise LauncherError("required launcher icon is missing")

    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        bundle = Path(temporary) / APP_NAME
        contents = bundle / "Contents"
        executable_directory = contents / "MacOS"
        resources = contents / "Resources"
        executable_directory.mkdir(parents=True)
        resources.mkdir()

        executable = executable_directory / "launch"
        executable.write_text(launcher_script(), encoding="utf-8")
        executable.chmod(0o755)
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump(info_plist(), stream, sort_keys=True)
        icon_builder(ICON_SOURCE, resources / "AppIcon.icns")
        os.replace(bundle, destination)
    return destination


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, default=DEFAULT_DESTINATION)
    return parser


def main(arguments: Sequence[str] | None = None) -> int:
    parsed = build_parser().parse_args(arguments)
    try:
        installed = install(parsed.destination.expanduser())
    except (LauncherError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"installed {installed}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
