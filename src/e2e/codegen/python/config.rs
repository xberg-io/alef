//! Python conftest.py and pyproject.toml rendering.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{DependencyMode, E2eConfig};
use crate::e2e::fixture::FixtureGroup;

use super::helpers::resolve_module;

// ---------------------------------------------------------------------------
// pyproject.toml
// ---------------------------------------------------------------------------

/// Format a list of pre-quoted TOML entries as an inline array when there is at
/// most one element, and as a multi-line array (2-space indent, trailing comma
/// after the final element) when there is more than one. Matches the canonical
/// `pyproject-fmt` output so the prek hook does not rewrite the e2e
/// `pyproject.toml` on every regen.
fn format_toml_array(entries: &[String]) -> String {
    match entries.len() {
        0 => "[]".to_string(),
        1 => format!("[{}]", entries[0]),
        _ => {
            let inner = entries.iter().map(|e| format!("  {e},")).collect::<Vec<_>>().join("\n");
            format!("[\n{inner}\n]")
        }
    }
}

/// PEP 508 version specifiers that may legally lead a version string.  When a
/// caller passes an already-qualified version (e.g. `">=1.0"`, `"==1.4.0-rc.32"`,
/// `"~=2.0"`), we leave it intact; when they pass a bare version (`"1.4.0-rc.32"`)
/// we prepend `==` so the resulting PEP 508 requirement is valid.
const PEP508_COMPARATORS: &[&str] = &["==", "!=", ">=", "<=", "~=", "===", ">", "<"];

fn normalize_python_version(pkg_version: &str) -> String {
    let trimmed = pkg_version.trim_start();
    if PEP508_COMPARATORS.iter().any(|c| trimmed.starts_with(c)) {
        pkg_version.to_string()
    } else {
        format!("=={pkg_version}")
    }
}

pub(super) fn render_pyproject(pkg_name: &str, pkg_path: &str, pkg_version: &str, dep_mode: DependencyMode) -> String {
    let (deps_line, uv_sources_block) = match dep_mode {
        DependencyMode::Registry => {
            let normalized_version = normalize_python_version(pkg_version);
            let entries = vec![
                format!("\"{pkg_name}{normalized_version}\""),
                "\"pytest>=7.4\"".to_string(),
                "\"pytest-asyncio>=0.23\"".to_string(),
                "\"pytest-timeout>=2.1\"".to_string(),
            ];
            (format!("dependencies = {}", format_toml_array(&entries)), String::new())
        }
        DependencyMode::Local => {
            let entries = vec![
                format!("\"{pkg_name}\""),
                "\"pytest>=7.4\"".to_string(),
                "\"pytest-asyncio>=0.23\"".to_string(),
                "\"pytest-timeout>=2.1\"".to_string(),
            ];
            (
                format!("dependencies = {}", format_toml_array(&entries)),
                format!(
                    "\n[tool.uv]\nsources.{pkg_name} = {{ path = \"{pkg_path}\" }}\n",
                    pkg_path = pkg_path
                ),
            )
        }
    };

    let requires_array = format_toml_array(&["\"setuptools>=68\"".to_string(), "\"wheel\"".to_string()]);
    let ruff_ignore_array = format_toml_array(&["\"PLR2004\"".to_string()]);
    let ruff_tests_array = format_toml_array(&[
        "\"B017\"".to_string(),
        "\"PT011\"".to_string(),
        "\"S101\"".to_string(),
        "\"S108\"".to_string(),
    ]);
    let pytest_testpaths_array = format_toml_array(&["\"tests\"".to_string()]);

    format!(
        r#"[build-system]
build-backend = "setuptools.build_meta"
requires = {requires_array}

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
packages = []
{uv_sources_block}
[tool.ruff]
lint.ignore = {ruff_ignore_array}
lint.per-file-ignores."tests/**" = {ruff_tests_array}

[tool.pytest]
ini_options.asyncio_mode = "auto"
ini_options.testpaths = {pytest_testpaths_array}
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
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let python_override = cc
            .overrides
            .get("python")
            .or_else(|| e2e_config.call.overrides.get("python"));
        python_override.and_then(|o| o.client_factory.as_deref()).is_some()
    });

    let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
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
    # Read startup lines from the mock server.  The server emits at most two:
    #   MOCK_SERVER_URL=http://...
    #   MOCK_SERVERS={{"fixture_id":"http://..."}}  (only when host-root fixtures exist)
    # We read up to 8 lines and stop as soon as we have seen MOCK_SERVER_URL and either
    # MOCK_SERVERS or a line that does not start with "MOCK_SERVER".
    for _ in range(8):
        raw_line = proc.stdout.readline()
        if not raw_line:
            break
        line = raw_line.decode().strip()
        if line.startswith("MOCK_SERVER_URL="):
            url = line.split("=", 1)[1]
        elif line.startswith("MOCK_SERVERS="):
            import json as _json  # noqa: PLC0415

            _json_val = line.split("=", 1)[1]
            os.environ["MOCK_SERVERS"] = _json_val
            for _fid, _furl in _json.loads(_json_val).items():
                os.environ[f"MOCK_SERVER_{{_fid.upper()}}"] = _furl
            break
        elif url:
            # We have the URL and this line is not MOCK_SERVERS — we are done.
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
        let test_documents_dir = &e2e_config.test_documents_dir;
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
import os
from pathlib import Path

# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.

# Change to the configured test-documents directory so that fixture file
# paths like "pdf/fake_memo.pdf" resolve correctly when running pytest
# from e2e/python/.
_TEST_DOCUMENTS = Path(__file__).parent.parent.parent / "{test_documents_dir}"
if _TEST_DOCUMENTS.is_dir():
    os.chdir(_TEST_DOCUMENTS)

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
    use crate::e2e::config::DependencyMode;

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
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let groups: Vec<crate::e2e::fixture::FixtureGroup> = Vec::new();
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

    #[test]
    fn render_pyproject_canonical_format_local() {
        // Verify e2e pyproject.toml is emitted in pyproject-fmt canonical form:
        // - build-backend before requires in [build-system]
        // - multi-item arrays are one-element-per-line with 2-space indent and a
        //   trailing comma (matches the prek `pyproject-fmt` hook's output)
        // - dependencies in alphabetical order
        let out = render_pyproject("my-pkg", "../../packages/python", "==0.5.0", DependencyMode::Local);

        // Check build-system section has build-backend before requires
        let build_section = out.split("[project]").next().unwrap();
        let backend_idx = build_section.find("build-backend").unwrap();
        let requires_idx = build_section.find("requires").unwrap();
        assert!(
            backend_idx < requires_idx,
            "build-backend should come before requires in [build-system]"
        );

        // Multi-item arrays must use the multi-line, one-element-per-line shape
        // with a trailing comma — verified on `requires` (2 elements).
        assert!(
            out.contains("requires = [\n  \"setuptools>=68\",\n  \"wheel\",\n]"),
            "requires should be emitted as a multi-line array. got: {out}"
        );

        // Check dependencies are alphabetically sorted within the multi-line
        // dependencies = [ ... ] block.
        let deps_start = out.find("dependencies = [").expect("dependencies array");
        let deps_block = &out[deps_start..];
        let pkg_idx = deps_block.find("\"my-pkg\"").unwrap();
        let pytest_idx = deps_block.find("\"pytest>=7.4\"").unwrap();
        let asyncio_idx = deps_block.find("\"pytest-asyncio>=0.23\"").unwrap();
        let timeout_idx = deps_block.find("\"pytest-timeout>=2.1\"").unwrap();
        assert!(
            pkg_idx < pytest_idx && pytest_idx < asyncio_idx && asyncio_idx < timeout_idx,
            "dependencies should be alphabetically sorted: my-pkg, pytest, pytest-asyncio, pytest-timeout. got: {deps_block}",
        );
    }

    #[test]
    fn render_pyproject_canonical_format_registry() {
        // Similar test for registry mode (no uv sources)
        let out = render_pyproject("my-pkg", "../../packages/python", "==0.5.0", DependencyMode::Registry);

        // Multi-item arrays must use the multi-line, one-element-per-line shape.
        assert!(
            out.contains("requires = [\n  \"setuptools>=68\",\n  \"wheel\",\n]"),
            "requires should be emitted as a multi-line array. got: {out}"
        );

        // Dependencies include package version specifier and are sorted.
        let deps_start = out.find("dependencies = [").expect("dependencies array");
        let deps_block = &out[deps_start..];
        assert!(
            deps_block.contains("\"my-pkg==0.5.0\""),
            "registry mode should include version specifier. got: {deps_block}",
        );
        let pkg_idx = deps_block.find("\"my-pkg==0.5.0\"").unwrap();
        let pytest_idx = deps_block.find("\"pytest>=7.4\"").unwrap();
        assert!(
            pkg_idx < pytest_idx,
            "my-pkg should come before pytest. got: {deps_block}",
        );
    }

    #[test]
    fn render_pyproject_registry_bare_version_gets_eq_comparator() {
        // When a caller passes a bare version (no PEP 508 comparator) we
        // auto-prepend `==` so the resulting requirement is a valid PEP 508
        // specifier. Bare `1.4.0-rc.30` previously rendered as
        // `"my-pkg1.4.0-rc.30"` which pip/uv reject.
        let out = render_pyproject(
            "my-pkg",
            "../../packages/python",
            "1.4.0-rc.30",
            DependencyMode::Registry,
        );
        let deps_start = out.find("dependencies = [").expect("dependencies array");
        let deps_block = &out[deps_start..];
        assert!(
            deps_block.contains("\"my-pkg==1.4.0-rc.30\""),
            "bare version should be normalised to `==1.4.0-rc.30`. got: {deps_block}",
        );
    }

    #[test]
    fn render_pyproject_registry_preserves_existing_comparator() {
        // Already-qualified versions pass through unchanged.
        for spec in ["==1.2.3", ">=1.0", "~=2.0", "!=1.5", "<3.0", ">1.0"] {
            let out = render_pyproject("my-pkg", "../../packages/python", spec, DependencyMode::Registry);
            assert!(
                out.contains(&format!("\"my-pkg{spec}\"")),
                "version `{spec}` should pass through unchanged. got: {out}",
            );
        }
    }

    #[test]
    fn render_pyproject_single_element_arrays_stay_inline() {
        // Single-element arrays must remain on one line (e.g. `lint.ignore = ["PLR2004"]`).
        let out = render_pyproject("my-pkg", "../../packages/python", ">=0.1.0", DependencyMode::Local);
        assert!(
            out.contains("lint.ignore = [\"PLR2004\"]"),
            "single-element arrays should stay inline. got: {out}"
        );
        assert!(
            out.contains("ini_options.testpaths = [\"tests\"]"),
            "single-element testpaths should stay inline. got: {out}"
        );
    }
}
