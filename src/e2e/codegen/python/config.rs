//! Python conftest.py and pyproject.toml rendering.

use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::core::version::to_pep440;
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
        format!("=={}", to_pep440(pkg_version))
    }
}

pub(super) fn render_pyproject(pkg_name: &str, pkg_path: &str, pkg_version: &str, dep_mode: DependencyMode) -> String {
    let (deps_line, uv_sources_block) = match dep_mode {
        DependencyMode::Registry => {
            let normalized_version = normalize_python_version(pkg_version);
            let entries = vec![
                format!("\"{pkg_name}{normalized_version}\""),
                format!("\"pytest{}\"", tv::pypi::PYTEST),
                format!("\"pytest-asyncio{}\"", tv::pypi::PYTEST_ASYNCIO),
                format!("\"pytest-timeout{}\"", tv::pypi::PYTEST_TIMEOUT),
            ];
            (format!("dependencies = {}", format_toml_array(&entries)), String::new())
        }
        DependencyMode::Local => {
            let entries = vec![
                format!("\"{pkg_name}\""),
                format!("\"pytest{}\"", tv::pypi::PYTEST),
                format!("\"pytest-asyncio{}\"", tv::pypi::PYTEST_ASYNCIO),
                format!("\"pytest-timeout{}\"", tv::pypi::PYTEST_TIMEOUT),
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

    let requires_array = format_toml_array(&[
        format!("\"setuptools{}\"", tv::pypi::SETUPTOOLS),
        "\"wheel\"".to_string(),
    ]);
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

[tool.pytest.ini_options]
asyncio_mode = "auto"
testpaths = {pytest_testpaths_array}
python_files = "test_*.py"
python_functions = "test_*"
addopts = "-v --strict-markers --tb=short"
timeout = 300
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
///
/// NOTE: called only by `render_app_harness` which is kept for tests only.
#[allow(dead_code)]
fn build_middleware_value(middleware: &Option<crate::e2e::fixture::HttpMiddleware>) -> serde_json::Value {
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
///
/// NOTE: production emission is handled by a consumer extension
/// (which copies this logic verbatim). This function is kept for tests only.
#[allow(dead_code)]
pub(super) fn render_app_harness(
    e2e_config: &E2eConfig,
    groups: &[FixtureGroup],
    crate_config: &crate::core::config::ResolvedCrateConfig,
) -> String {
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
    let app_class = e2e_config.harness.app_class_for_lang("python");
    // Python is snake_case-native; `register_method_idiomatic` preserves
    // the canonical name verbatim for python.
    let register_route_method = e2e_config
        .harness
        .register_method_idiomatic("python")
        .unwrap_or_else(|| "register_route".to_string());
    let body_schema_setter = &e2e_config.harness.body_schema_setter;
    let method_enum = &e2e_config.harness.method_enum;
    let run_method = e2e_config.harness.run_method_for_lang("python");
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

    // Check if App.config is excluded from bindings
    let skip_app_config = crate_config.exclude.methods.iter().any(|m| m == "App.config");

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
        register_route_method => register_route_method.as_str(),
        run_method => run_method.as_deref().unwrap_or("run"),
        response_body_field => e2e_config.harness.response_body_field.as_str(),
        host => host,
        port => port,
        fixtures_json => fixtures_json,
        skip_app_config => skip_app_config,
    };

    crate::e2e::template_env::render("python/app_harness.py.jinja", ctx)
}

// ---------------------------------------------------------------------------
// conftest.py
// ---------------------------------------------------------------------------

/// Emit a Python snippet that copies every `[e2e.env]` entry into `os.environ`
/// using `setdefault` so a parent runner can still override at spawn time.
/// Returns empty when no env vars are configured.
fn render_env_setup_block(e2e_config: &E2eConfig) -> String {
    if e2e_config.env.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = e2e_config.env.keys().collect();
    keys.sort();
    let entries = keys
        .iter()
        .map(|k| format!("    {:?}: {:?},", k, &e2e_config.env[*k]))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "import os as _os\n\n_SUITE_ENV = {{\n{entries}\n}}\nfor _k, _v in _SUITE_ENV.items():\n    _os.environ.setdefault(_k, _v)\n\n"
    )
}

pub(super) fn render_conftest(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    let module = resolve_module(e2e_config);

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
    let env_setup = render_env_setup_block(e2e_config);

    // NOTE: when uses_harness is true (server-pattern), the conftest.py is emitted
    // by a consumer extension (Extension::emit_e2e "python" arm).
    // alef falls through to the client/mock-server or minimal conftest below.
    if has_mock_server_fixtures {
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

{env_setup}# Ensure the package is importable.
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
        # Expand the parent-set MOCK_SERVERS JSON into per-fixture
        # MOCK_SERVER_<FIXTURE_ID> env vars so tests reading
        # `MOCK_SERVER_<UPPER>` find the dedicated per-fixture URL
        # (without this, tests fall back to the shared-server namespaced
        # URL where origin-relative asset paths 404).
        _mock_servers = os.environ.get("MOCK_SERVERS")
        if _mock_servers:
            import json as _json  # noqa: PLC0415

            for _fid, _furl in _json.loads(_mock_servers).items():
                os.environ[f"MOCK_SERVER_{{_fid.upper()}}"] = _furl
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

{env_setup}# Ensure the package is importable.
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
{env_setup}# Ensure the package is importable.
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
    fn render_conftest_emits_env_block_when_env_configured() {
        let mut e2e_config = crate::e2e::config::E2eConfig::default();
        e2e_config
            .env
            .insert("E2E_ALLOW_PRIVATE_NETWORK".to_owned(), "true".to_owned());
        e2e_config.env.insert("ALEF_FOO".to_owned(), "bar".to_owned());
        let groups: Vec<crate::e2e::fixture::FixtureGroup> = Vec::new();
        let out = render_conftest(&e2e_config, &groups);
        assert!(out.contains("_SUITE_ENV"), "got: {out}");
        assert!(out.contains("\"E2E_ALLOW_PRIVATE_NETWORK\""), "got: {out}");
        assert!(out.contains("\"ALEF_FOO\""), "got: {out}");
        assert!(out.contains("setdefault"), "got: {out}");
        let alef_pos = out.find("\"ALEF_FOO\"").unwrap();
        let e2e_pos = out.find("\"E2E_ALLOW_PRIVATE_NETWORK\"").unwrap();
        assert!(alef_pos < e2e_pos, "keys should be sorted alphabetically; got: {out}");
    }

    #[test]
    fn render_conftest_skips_env_block_when_env_empty() {
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let groups: Vec<crate::e2e::fixture::FixtureGroup> = Vec::new();
        let out = render_conftest(&e2e_config, &groups);
        assert!(
            !out.contains("_SUITE_ENV"),
            "empty env should emit no block; got: {out}"
        );
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
    fn render_pyproject_registry_bare_version_gets_pep440_eq_comparator() {
        // Bare SemVer pre-release `1.4.0-rc.30` must be normalised to PEP 440
        // canonical form `==1.4.0rc30` — the dash form is not valid on PyPI.
        let out = render_pyproject(
            "my-pkg",
            "../../packages/python",
            "1.4.0-rc.30",
            DependencyMode::Registry,
        );
        let deps_start = out.find("dependencies = [").expect("dependencies array");
        let deps_block = &out[deps_start..];
        assert!(
            deps_block.contains("\"my-pkg==1.4.0rc30\""),
            "bare pre-release version should be normalised to PEP 440 `==1.4.0rc30`. got: {deps_block}",
        );
        assert!(
            !deps_block.contains("1.4.0-rc.30"),
            "raw semver dash form must not appear in pyproject.toml deps. got: {deps_block}",
        );
    }

    #[test]
    fn render_pyproject_registry_rc_prerelease_is_pep440_canonical() {
        // The canonical fix: 3.6.0-rc.1 → ==3.6.0rc1
        let out = render_pyproject(
            "my-pkg",
            "../../packages/python",
            "3.6.0-rc.1",
            DependencyMode::Registry,
        );
        let deps_start = out.find("dependencies = [").expect("dependencies array");
        let deps_block = &out[deps_start..];
        assert!(
            deps_block.contains("\"my-pkg==3.6.0rc1\""),
            "3.6.0-rc.1 must render as ==3.6.0rc1. got: {deps_block}"
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
            out.contains("testpaths = [\"tests\"]"),
            "single-element testpaths should stay inline. got: {out}"
        );
    }

    #[test]
    fn render_app_harness_emits_options_preflight_handler_for_cors_fixtures() {
        use crate::core::config::e2e::{E2eConfig, HarnessConfig};
        use crate::core::config::resolved::ResolvedCrateConfig;
        use crate::e2e::fixture::{
            CorsConfig, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpMiddleware,
            HttpRequest,
        };
        use std::collections::BTreeMap;

        let cors_fixture = Fixture {
            id: "cors_preflight_test".to_owned(),
            description: "CORS preflight test".to_owned(),
            category: Some("cors".to_owned()),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test".to_owned(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/api/test".to_owned(),
                    method: "GET".to_owned(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: Some(HttpMiddleware {
                        cors: Some(CorsConfig {
                            allow_origins: vec!["https://example.com".to_owned()],
                            allow_methods: vec!["GET".to_owned(), "POST".to_owned()],
                            allow_headers: vec!["Content-Type".to_owned()],
                            expose_headers: vec![],
                            max_age: Some(3600),
                            allow_credentials: false,
                        }),
                        ..Default::default()
                    }),
                },
                request: HttpRequest {
                    method: "OPTIONS".to_owned(),
                    path: "/api/test".to_owned(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 204,
                    body: None,
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
        };

        let groups = vec![FixtureGroup {
            category: "cors".to_owned(),
            fixtures: vec![cors_fixture],
        }];
        let e2e_config = E2eConfig {
            harness: HarnessConfig {
                imports: vec!["my_pkg".to_owned()],
                ..HarnessConfig::default()
            },
            ..E2eConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();

        let out = render_app_harness(&e2e_config, &groups, &crate_config);

        // The harness must emit an OPTIONS preflight handler for the CORS-enabled fixture
        assert!(
            out.contains("make_cors_preflight_handler"),
            "expected `make_cors_preflight_handler` in generated app_harness.py for CORS fixture:\n{out}"
        );
        assert!(
            out.contains("options_builder"),
            "expected `options_builder` registration in generated app_harness.py:\n{out}"
        );
        assert!(
            out.contains("OPTIONS"),
            "expected `OPTIONS` method enum variant in generated app_harness.py:\n{out}"
        );
    }

    #[test]
    fn render_app_harness_skips_app_config_when_excluded() {
        use crate::core::config::e2e::{E2eConfig, HarnessConfig};
        use crate::core::config::resolved::ResolvedCrateConfig;
        use crate::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
        use std::collections::BTreeMap;

        let fixture = Fixture {
            id: "simple_test".to_owned(),
            description: "Simple test".to_owned(),
            category: Some("smoke".to_owned()),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test".to_owned(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/test".to_owned(),
                    method: "GET".to_owned(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "GET".to_owned(),
                    path: "/test".to_owned(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: None,
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
        };

        let groups = vec![FixtureGroup {
            category: "smoke".to_owned(),
            fixtures: vec![fixture],
        }];
        let e2e_config = E2eConfig {
            harness: HarnessConfig {
                imports: vec!["my_pkg".to_owned()],
                ..HarnessConfig::default()
            },
            ..E2eConfig::default()
        };

        // Create a config with App.config excluded
        let mut crate_config = ResolvedCrateConfig::default();
        crate_config.exclude.methods = vec!["App.config".to_owned()];

        let out = render_app_harness(&e2e_config, &groups, &crate_config);

        // When App.config is excluded, the harness should NOT emit the app.config() call
        assert!(
            !out.contains("app.config(_config)"),
            "expected app.config(_config) to be skipped when App.config is excluded, but it was emitted:\n{out}"
        );
        // But it should still print the listening message and call run
        assert!(
            out.contains("Harness listening on"),
            "expected listening message in harness:\n{out}"
        );
        assert!(out.contains("app.run()"), "expected app.run() call in harness:\n{out}");
    }
}
