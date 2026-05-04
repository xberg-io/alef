//! Python conftest.py and pyproject.toml rendering.

use crate::config::{DependencyMode, E2eConfig};
use crate::fixture::FixtureGroup;
use alef_core::hash::{self, CommentStyle};

use super::helpers::resolve_module;

// ---------------------------------------------------------------------------
// pyproject.toml
// ---------------------------------------------------------------------------

pub(super) fn render_pyproject(pkg_name: &str, pkg_path: &str, pkg_version: &str, dep_mode: DependencyMode) -> String {
    let (deps_line, uv_sources_block) = match dep_mode {
        DependencyMode::Registry => (
            format!(
                "dependencies = [ \"pytest>=7.4\", \"pytest-asyncio>=0.23\", \
                 \"pytest-timeout>=2.1\", \"{pkg_name}{pkg_version}\" ]"
            ),
            String::new(),
        ),
        DependencyMode::Local => (
            format!(
                "dependencies = [ \"pytest>=7.4\", \"pytest-asyncio>=0.23\", \
                 \"pytest-timeout>=2.1\", \"{pkg_name}\" ]"
            ),
            format!(
                "\n[tool.uv]\nsources.{pkg_name} = {{ path = \"{pkg_path}\" }}\n",
                pkg_path = pkg_path
            ),
        ),
    };

    format!(
        r#"[build-system]
build-backend = "setuptools.build_meta"
requires = [ "setuptools>=68", "wheel" ]

[project]
name = "{pkg_name}-e2e"
version = "0.0.0"
description = "End-to-end tests"
requires-python = ">=3.10"
classifiers = [
  "Programming Language :: Python :: 3 :: Only",
  "Programming Language :: Python :: 3.10",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Programming Language :: Python :: 3.13",
  "Programming Language :: Python :: 3.14",
]
{deps_line}

[tool.setuptools]
packages = [  ]
{uv_sources_block}
[tool.ruff]
lint.ignore = [ "PLR2004" ]
lint.per-file-ignores."tests/**" = [ "B017", "PT011", "S101", "S108" ]

[tool.pytest]
ini_options.asyncio_mode = "auto"
ini_options.testpaths = [ "tests" ]
ini_options.python_files = "test_*.py"
ini_options.python_functions = "test_*"
ini_options.addopts = "-v --strict-markers --tb=short"
ini_options.timeout = 300
"#
    )
}

// ---------------------------------------------------------------------------
// conftest.py
// ---------------------------------------------------------------------------

pub(super) fn render_conftest(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    let module = resolve_module(e2e_config);
    let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
        if f.needs_mock_server() {
            return true;
        }
        let cc = e2e_config.resolve_call(f.call.as_deref());
        let python_override = cc
            .overrides
            .get("python")
            .or_else(|| e2e_config.call.overrides.get("python"));
        python_override.and_then(|o| o.client_factory.as_deref()).is_some()
    });

    let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
        let cc = e2e_config.resolve_call(f.call.as_deref());
        cc.args
            .iter()
            .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
    });

    let header = hash::header(CommentStyle::Hash);
    if has_http_fixtures {
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
from __future__ import annotations

import os
import subprocess
import threading
from pathlib import Path
from typing import Generator

import pytest

# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.

_HERE = Path(__file__).parent
_E2E_DIR = _HERE.parent
_MOCK_SERVER_BIN = _E2E_DIR / "rust" / "target" / "release" / "mock-server"
_FIXTURES_DIR = _E2E_DIR.parent / "fixtures"


@pytest.fixture(scope="session", autouse=True)
def mock_server() -> Generator[str, None, None]:
    """Spawn the mock HTTP server binary and set MOCK_SERVER_URL."""
    proc = subprocess.Popen(  # noqa: S603
        [str(_MOCK_SERVER_BIN), str(_FIXTURES_DIR)],
        stdout=subprocess.PIPE,
        stderr=None,
        stdin=subprocess.PIPE,
    )
    url = ""
    assert proc.stdout is not None
    for raw_line in proc.stdout:
        line = raw_line.decode().strip()
        if line.startswith("MOCK_SERVER_URL="):
            url = line.split("=", 1)[1]
            break
    os.environ["MOCK_SERVER_URL"] = url
    # Drain stdout in background so the server never blocks.
    threading.Thread(target=proc.stdout.read, daemon=True).start()
    yield url
    if proc.stdin:
        proc.stdin.close()
    proc.terminate()
    proc.wait()


def _make_request(method: str, path: str, **kwargs: object) -> object:
    """Make an HTTP request to the mock server."""
    import urllib.request  # noqa: PLC0415

    base_url = os.environ.get("MOCK_SERVER_URL", "http://localhost:8080")
    url = f"{{base_url}}{{path}}"
    data = kwargs.pop("json", None)
    if data is not None:
        import json  # noqa: PLC0415

        body = json.dumps(data).encode()
        headers = dict(kwargs.pop("headers", {{}}))
        headers.setdefault("Content-Type", "application/json")
        req = urllib.request.Request(url, data=body, headers=headers, method=method.upper())
    else:
        headers = dict(kwargs.pop("headers", {{}}))
        req = urllib.request.Request(url, headers=headers, method=method.upper())
    try:
        with urllib.request.urlopen(req) as resp:  # noqa: S310
            return resp
    except urllib.error.HTTPError as exc:
        return exc


@pytest.fixture(scope="session")
def app(mock_server: str) -> object:  # noqa: ARG001
    """Return a simple HTTP helper bound to the mock server URL."""

    class _App:
        def request(self, path: str, **kwargs: object) -> object:
            method = str(kwargs.pop("method", "GET"))
            return _make_request(method, path, **kwargs)

    return _App()
"#
        )
    } else if has_file_fixtures {
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
import os
from pathlib import Path

# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.

# Change to the test_documents directory so that fixture file paths like
# "pdf/fake_memo.pdf" resolve correctly when running pytest from e2e/python/.
_TEST_DOCUMENTS = Path(__file__).parent.parent.parent / "test_documents"
if _TEST_DOCUMENTS.is_dir():
    os.chdir(_TEST_DOCUMENTS)

# On macOS, Pdfium is a separate dylib not on the default library path in dev builds.
# Search common locations (Cargo build output, staged target/release) and extend
# DYLD_LIBRARY_PATH / LD_LIBRARY_PATH so the extension can load the library.
_REPO_ROOT = Path(__file__).parent.parent.parent


def _find_pdfium_dir() -> str | None:
    """Find the directory containing libpdfium, searching Cargo build outputs."""
    for _candidate in sorted(_REPO_ROOT.glob("target/*/release/build/*/out/libpdfium*")):
        return str(_candidate.parent)
    for _candidate in sorted(_REPO_ROOT.glob("target/release/build/*/out/libpdfium*")):
        return str(_candidate.parent)
    return None


_pdfium_dir = _find_pdfium_dir()
if _pdfium_dir is not None:
    for _var in ("DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"):
        _existing = os.environ.get(_var, "")
        if _pdfium_dir not in _existing:
            os.environ[_var] = f"{{_pdfium_dir}}:{{_existing}}" if _existing else _pdfium_dir
"#
        )
    } else {
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.
"#
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DependencyMode;

    #[test]
    fn render_pyproject_local_uses_uv_sources_block() {
        let out = render_pyproject("my-pkg", "../../packages/python", ">=0.1.0", DependencyMode::Local);
        assert!(out.contains("[tool.uv]"), "got: {out}");
        assert!(out.contains("sources.my-pkg"), "got: {out}");
    }

    #[test]
    fn render_pyproject_registry_omits_uv_sources_block() {
        let out = render_pyproject("my-pkg", "../../packages/python", ">=0.1.0", DependencyMode::Registry);
        assert!(!out.contains("[tool.uv]"), "got: {out}");
        assert!(out.contains("my-pkg>=0.1.0"), "got: {out}");
    }

    #[test]
    fn render_conftest_no_fixtures_emits_minimal_conftest() {
        let e2e_config = crate::config::E2eConfig::default();
        let groups: Vec<crate::fixture::FixtureGroup> = Vec::new();
        let out = render_conftest(&e2e_config, &groups);
        assert!(out.contains("Pytest configuration"), "got: {out}");
        assert!(!out.contains("mock_server"), "got: {out}");
    }

    #[test]
    fn render_pyproject_contains_project_section() {
        let out = render_pyproject("my-pkg", "../../packages/python", ">=0.1.0", DependencyMode::Local);
        assert!(out.contains("[project]"), "got: {out}");
        assert!(out.contains("my-pkg-e2e"), "got: {out}");
    }
}
