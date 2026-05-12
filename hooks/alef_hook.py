#!/usr/bin/env python3
"""Resolve and exec the pinned alef binary for pre-commit hooks.

Resolution order:
    1. `alef` already on PATH whose `alef --version` matches the version pinned
       in this repo's `alef.toml`. Zero-cost path for developers who have run
       `cargo install --path crates/alef-cli`.
    2. Cached release tarball under `~/.cache/alef-hooks/{version}/`. If the
       cache is empty, downloads from GitHub releases and verifies sha256.

Both paths exec the resolved binary with the args passed in by the hook entry.
"""

import hashlib
import os
import platform
import shutil
import subprocess
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


def _parse_quoted(value: str) -> str:
    return value.strip().strip('"').strip("'")


def _read_cargo_version(cargo_toml: Path) -> str | None:
    """Read `[workspace.package] version` (or `[package] version`) from a Cargo.toml."""
    in_workspace_package = False
    in_package = False
    for line in cargo_toml.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_workspace_package = stripped == "[workspace.package]"
            in_package = stripped == "[package]"
            continue
        if (in_workspace_package or in_package) and stripped.startswith("version"):
            _, _, val = stripped.partition("=")
            return _parse_quoted(val)
    return None


def _version() -> str:
    alef_toml = _hooks_dir().parent / "alef.toml"
    # alef.toml's [crate] table may declare `version_from = "Cargo.toml"`, in which
    # case the authoritative version is the workspace Cargo.toml — the top-level
    # `version = "..."` field in alef.toml is a stale fallback that gets bumped lazily.
    version_from: str | None = None
    inline_version: str | None = None
    in_crate = False
    for raw in alef_toml.read_text().splitlines():
        stripped = raw.strip()
        if stripped.startswith("["):
            in_crate = stripped == "[crate]"
            continue
        if in_crate and stripped.startswith("version_from"):
            _, _, val = stripped.partition("=")
            version_from = _parse_quoted(val)
        elif not in_crate and stripped.startswith("version") and inline_version is None:
            _, _, val = stripped.partition("=")
            inline_version = _parse_quoted(val)
    if version_from:
        cargo_toml = alef_toml.parent / version_from
        if cargo_toml.is_file():
            resolved = _read_cargo_version(cargo_toml)
            if resolved:
                return resolved
        # Fall through to the inline version if the referenced file is missing.
    if inline_version:
        return inline_version
    msg = "Could not resolve version (no [crate].version_from, no top-level version)"
    raise SystemExit(msg)


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


def _cache_dir(version: str) -> Path:
    base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return base / "alef-hooks" / version


def _binary_name() -> str:
    return "alef.exe" if platform.system().lower() == "windows" else "alef"


def _download_and_extract(version: str, asset_name: str, fmt: str, cache: Path) -> None:
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


def _system_binary_matches(version: str) -> Path | None:
    """Return the path to a system `alef` whose `--version` output matches.

    Looks up `alef` (or `alef.exe` on Windows) on PATH and runs `alef --version`.
    Accepts the binary if the trailing whitespace-trimmed token equals the pinned
    version. Returns None on any failure (binary missing, wrong version, version
    command failed, timeout).
    """
    candidate = shutil.which(_binary_name())
    if candidate is None:
        return None
    try:
        result = subprocess.run(
            [candidate, "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if result.returncode != 0:
        return None
    # Typical output: "alef 0.15.8" — accept the last whitespace-separated token.
    last_token = (result.stdout.strip().split() or [""])[-1].lstrip("v")
    if last_token != version:
        return None
    return Path(candidate)


def _resolve_binary() -> Path:
    version = _version()

    # Prefer a pre-installed `alef` on PATH whose --version matches.
    system_binary = _system_binary_matches(version)
    if system_binary is not None:
        return system_binary

    system, machine = _detect_platform()
    asset_name, fmt = _asset_name_for(system, machine)
    cache = _cache_dir(version)
    target = asset_name.split(".", maxsplit=1)[0]
    binary = cache / target / _binary_name()

    if not binary.is_file():
        _download_and_extract(version, asset_name, fmt, cache)

    if not os.access(binary, os.X_OK):
        binary.chmod(binary.stat().st_mode | 0o111)

    return binary


def main() -> None:
    binary = _resolve_binary()
    args = sys.argv[1:]
    os.execv(str(binary), [str(binary), *args])


if __name__ == "__main__":
    main()
