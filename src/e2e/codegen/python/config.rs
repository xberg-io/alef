//! Python conftest.py and pyproject.toml rendering.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{DependencyMode, E2eConfig};
use crate::e2e::fixture::FixtureGroup;
use serde_json::json;

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
line-length = 120
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
// app_harness.py (server-pattern harness)
// ---------------------------------------------------------------------------

/// Convert the fixture's `HttpMiddleware` into a `serde_json::Value` suitable
/// for embedding in the harness fixture JSON.
///
/// Field names are normalised to match each binding's `from_json()` contract:
///
/// - CORS: alef fixture schema uses `allow_origins` / `allow_methods` /
///   `allow_headers` (without the `d`), while the binding's `CorsConfig`
///   deserialises `allowed_origins` / `allowed_methods` / `allowed_headers`.
///   Keys are renamed here so the template can call `CorsConfig.from_json()`
///   directly without any in-template remapping.
fn build_middleware_value(
    middleware: &Option<crate::e2e::fixture::HttpMiddleware>,
) -> serde_json::Value {
    let Some(mw) = middleware else {
        return serde_json::Value::Null;
    };

    let mut map = serde_json::Map::new();

    // --- cors ---
    if let Some(cors) = &mw.cors {
        let mut cors_map = serde_json::Map::new();
        // Remap allow_* → allowed_* to match the binding's CorsConfig.from_json().
        cors_map.insert("allowed_origins".to_string(), json!(cors.allow_origins));
        cors_map.insert("allowed_methods".to_string(), json!(cors.allow_methods));
        cors_map.insert("allowed_headers".to_string(), json!(cors.allow_headers));
        if !cors.expose_headers.is_empty() {
            cors_map.insert("expose_headers".to_string(), json!(cors.expose_headers));
        }
        if let Some(max_age) = cors.max_age {
            cors_map.insert("max_age".to_string(), json!(max_age));
        }
        if cors.allow_credentials {
            cors_map.insert("allow_credentials".to_string(), json!(true));
        }
        map.insert("cors".to_string(), serde_json::Value::Object(cors_map));
    }

    // --- pass-through middlewares (keys already match binding expectations) ---
    for (key, value) in [
        ("jwt_auth", &mw.jwt_auth),
        ("api_key_auth", &mw.api_key_auth),
        ("compression", &mw.compression),
        ("rate_limit", &mw.rate_limit),
        ("request_timeout", &mw.request_timeout),
        ("request_id", &mw.request_id),
    ] {
        if let Some(v) = value {
            map.insert(key.to_string(), v.clone());
        }
    }

    if map.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(map)
    }
}

/// Render the app harness script for server-pattern HTTP fixtures.
///
/// The harness spawns the SUT app and registers handlers per fixture,
/// returning canned expected responses. It's driven by conftest.py's
/// subprocess launcher.
pub(super) fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            // We only need the http field, handler, request, and expected_response.
            let http_data = &fixture.http.as_ref().unwrap();

            // Build the middleware map with normalised field names so the
            // harness template can call each config's `from_json()` directly.
            let middleware_value = build_middleware_value(&http_data.handler.middleware);

            let fixture_json = json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                        "middleware": middleware_value,
                    },
                    "request": {
                        "path": &http_data.request.path,
                    },
                    "expected_response": {
                        "status_code": http_data.expected_response.status_code,
                        "body": &http_data.expected_response.body,
                        "headers": &http_data.expected_response.headers,
                    }
                }
            });
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json = serde_json::to_string(&fixtures_map).unwrap_or_default();

    let imports = &e2e_config.harness.imports;
    let app_class = &e2e_config.harness.app_class;
    let register_route_method = &e2e_config.harness.register_method;
    let body_schema_setter = &e2e_config.harness.body_schema_setter;
    let method_enum = &e2e_config.harness.method_enum;
    let run_method = &e2e_config.harness.run_method;
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;

    let header = hash::header(CommentStyle::Hash);

    let route_builder_import = if !imports.is_empty() {
        let module_leaf = imports[0].rsplit('.').next().unwrap_or(&imports[0]).replace('-', "_");
        format!("{}._{}", &imports[0], module_leaf)
    } else {
        "app._app".to_string()
    };
    let method_enum_import = route_builder_import.clone();

    let ctx = minijinja::context! {
        header => header,
        imports => imports,
        app_class => app_class.as_deref().unwrap_or("App"),
        route_builder_import => route_builder_import,
        route_builder_class => "RouteBuilder",
        route_builder_constructor => "__init__",
        route_builder_schema_setter => body_schema_setter.as_deref().unwrap_or("request_schema_json"),
        method_enum_import => method_enum_import,
        method_enum_class => method_enum.as_deref().unwrap_or("Method"),
        register_route_method => register_route_method.as_deref().unwrap_or("register_route"),
        run_method => run_method.as_deref().unwrap_or("run"),
        response_body_field => e2e_config.harness.response_body_field.as_str(),
        host => host,
        port => port,
        fixtures_json => fixtures_json,
    };

    crate::e2e::template_env::render("python/app_harness.py.jinja", ctx)
}

// ---------------------------------------------------------------------------
// conftest.py
// ---------------------------------------------------------------------------

pub(super) fn render_conftest(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    let module = resolve_module(e2e_config);

    // Check for server-pattern HTTP fixtures (require harness).
    let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
    let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();

    let has_mock_server_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
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

    if uses_harness {
        // Server-pattern harness: spawn app_harness.py subprocess
        let host = &e2e_config.harness.host;
        let port = e2e_config.harness.port;
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
from __future__ import annotations

import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Generator

import pytest

# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.

_HERE = Path(__file__).parent
_APP_HARNESS = _HERE / "app_harness.py"


@pytest.fixture(scope="session", autouse=True)
def sut_server() -> Generator[str, None, None]:
    """Spawn the app harness and set SUT_URL.

    If SUT_URL is already set, a parent process started a shared harness.
    Use it as-is and do NOT spawn our own.
    """
    import socket  # noqa: PLC0415

    existing = os.environ.get("SUT_URL")
    if existing:
        yield existing
        return

    # Spawn the harness script as a subprocess.
    proc = subprocess.Popen(
        [sys.executable, str(_APP_HARNESS)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        stdin=subprocess.PIPE,
    )

    url = f"http://{host}:{port}"
    # Poll until the harness actually accepts TCP connections. The harness
    # may print a listening banner before the runtime has finished binding,
    # so port availability is the authoritative readiness signal.
    deadline = time.time() + 15.0
    ready = False
    while time.time() < deadline:
        if proc.poll() is not None:
            # Process died early; surface stderr in the failure path.
            break
        try:
            with socket.create_connection(("{host}", {port}), timeout=0.5):
                ready = True
                break
        except OSError:
            time.sleep(0.1)

    if not ready:
        stderr_bytes = proc.stderr.read() if proc.stderr else b""
        proc.terminate()
        raise RuntimeError(
            f"App harness did not become reachable on {host}:{port} within 15s; "
            f"stderr={{stderr_bytes[:1000]!r}}"
        )

    os.environ["SUT_URL"] = url
    yield url

    # Cleanup
    if proc.stdin:
        proc.stdin.close()
    proc.terminate()
    proc.wait(timeout=5)


@pytest.fixture(scope="session")
def app(sut_server: str) -> object:
    """Return a simple HTTP helper bound to the SUT server URL."""

    class _App:
        def request(self, path: str, **kwargs: object) -> object:
            import urllib.request  # noqa: PLC0415
            method = str(kwargs.pop("method", "GET"))
            url = f"{{sut_server}}{{path}}"
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

    return _App()
"#
        )
    } else if has_mock_server_fixtures {
        // Mock-server pattern (non-HTTP fixtures)
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
    """Spawn the mock HTTP server binary and set MOCK_SERVER_URL.

    If MOCK_SERVER_URL is already set, a parent process (e.g. `alef test-apps
    run`) started a shared mock-server and exported its URL (plus any
    MOCK_SERVERS / MOCK_SERVER_<FIXTURE_ID> vars). Use it as-is and do NOT spawn
    our own server.
    """
    existing = os.environ.get("MOCK_SERVER_URL")
    if existing:
        yield existing
        return
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
