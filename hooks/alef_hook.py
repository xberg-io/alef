#!/usr/bin/env python3
"""Download, verify, cache, and exec the pinned alef binary for pre-commit hooks."""

import hashlib
import os
import platform
import sys
import tarfile
import zipfile
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import urlretrieve

REPO = "kreuzberg-dev/alef"

_PLATFORM_MAP: dict[tuple[str, str], tuple[str, str]] = {
    ("linux", "x86_64"): ("alef-x86_64-unknown-linux-gnu.tar.gz", "tar.gz"),
    ("linux", "aarch64"): ("alef-aarch64-unknown-linux-gnu.tar.gz", "tar.gz"),
    ("darwin", "arm64"): ("alef-aarch64-apple-darwin.tar.gz", "tar.gz"),
    ("windows", "amd64"): ("alef-x86_64-pc-windows-gnu.zip", "zip"),
}


def _detect_platform() -> tuple[str, str]:
    system = platform.system().lower()
    machine = platform.machine().lower()
    if system == "darwin" and machine == "x86_64":
        msg = "macOS x86_64 is not supported by alef pre-built binaries"
        raise SystemExit(msg)
    return system, machine


def _asset_name_for(system: str, machine: str) -> tuple[str, str]:
    key = (system, machine)
    if key not in _PLATFORM_MAP:
        msg = f"Unsupported platform: {system}/{machine}"
        raise SystemExit(msg)
    return _PLATFORM_MAP[key]


def _hooks_dir() -> Path:
    return Path(__file__).parent


def _version() -> str:
    return (_hooks_dir() / "VERSION").read_text().strip()


def _expected_checksum(asset_name: str) -> str | None:
    checksums_file = _hooks_dir() / "checksums.txt"
    if not checksums_file.exists():
        return None
    for raw in checksums_file.read_text().splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        parts = stripped.split()
        if len(parts) == 2 and parts[1] == asset_name:  # noqa: PLR2004
            return parts[0]
    return None


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _cache_dir(version: str, asset_name: str) -> Path:
    target = asset_name.split(".", maxsplit=1)[0]
    base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return base / "alef-hooks" / version / target


def _binary_name() -> str:
    return "alef.exe" if platform.system().lower() == "windows" else "alef"


def _download_and_extract(version: str, asset_name: str, fmt: str, cache: Path) -> Path:
    url = f"https://github.com/{REPO}/releases/download/v{version}/{asset_name}"
    archive = cache / asset_name
    cache.mkdir(parents=True, exist_ok=True)

    print(f"[alef-hook] Downloading {url}", file=sys.stderr)
    try:
        urlretrieve(url, archive)
    except HTTPError as exc:
        raise SystemExit(
            f"Failed to download {asset_name} (HTTP {exc.code})\n  {url}\nEnsure v{version} release exists with assets."
        ) from None
    except URLError as exc:
        msg = f"Network error downloading {asset_name}: {exc.reason}"
        raise SystemExit(msg) from None

    expected = _expected_checksum(asset_name)
    if expected is not None:
        actual = _sha256(archive)
        if actual != expected:
            archive.unlink(missing_ok=True)
            msg = f"Checksum mismatch for {asset_name}: expected {expected}, got {actual}"
            raise SystemExit(msg)

    if fmt == "tar.gz":
        with tarfile.open(archive, "r:gz") as tf:
            tf.extractall(cache, filter="data")
    else:
        with zipfile.ZipFile(archive, "r") as zf:
            zf.extractall(cache)  # noqa: S202

    archive.unlink(missing_ok=True)
    return cache / _binary_name()


def _resolve_binary() -> Path:
    version = _version()
    system, machine = _detect_platform()
    asset_name, fmt = _asset_name_for(system, machine)
    cache = _cache_dir(version, asset_name)
    binary = cache / _binary_name()

    if not binary.is_file():
        binary = _download_and_extract(version, asset_name, fmt, cache)

    if not os.access(binary, os.X_OK):
        binary.chmod(binary.stat().st_mode | 0o111)

    return binary


def main() -> None:
    binary = _resolve_binary()
    args = sys.argv[1:]
    os.execv(str(binary), [str(binary), *args])


if __name__ == "__main__":
    main()
