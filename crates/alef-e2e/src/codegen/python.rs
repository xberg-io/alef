//! Python e2e test code generator.
//!
//! Generates `e2e/python/conftest.py` and `tests/test_{category}.py` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::codegen::resolve_field;
use crate::config::E2eConfig;
use crate::escape::{escape_python, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use anyhow::Result;
use heck::{ToShoutySnakeCase, ToSnakeCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

/// Python e2e test code generator.
pub struct PythonE2eCodegen;

impl super::E2eCodegen for PythonE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("python");

        // conftest.py
        files.push(GeneratedFile {
            path: output_base.join("conftest.py"),
            content: render_conftest(e2e_config, groups),
            generated_header: true,
        });

        // Root __init__.py (prevents ruff INP001).
        files.push(GeneratedFile {
            path: output_base.join("__init__.py"),
            content: String::new(),
            generated_header: false,
        });

        // tests/__init__.py
        files.push(GeneratedFile {
            path: output_base.join("tests").join("__init__.py"),
            content: String::new(),
            generated_header: false,
        });

        // pyproject.toml for standalone uv resolution
        let python_pkg = e2e_config.resolve_package("python");
        let pkg_name = python_pkg
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .unwrap_or("kreuzcrawl");
        let pkg_path = python_pkg
            .as_ref()
            .and_then(|p| p.path.as_deref())
            .unwrap_or("../../packages/python");
        let pkg_version = python_pkg
            .as_ref()
            .and_then(|p| p.version.as_deref())
            .unwrap_or("0.1.0");
        files.push(GeneratedFile {
            path: output_base.join("pyproject.toml"),
            content: render_pyproject(pkg_name, pkg_path, pkg_version, e2e_config.dep_mode),
            generated_header: true,
        });

        // Per-category test files.
        for group in groups {
            let fixtures: Vec<&Fixture> = group.fixtures.iter().collect();

            if fixtures.is_empty() {
                continue;
            }

            // Skip emitting the file entirely when every fixture is skipped for
            // python — there's nothing to run, and emitting imports of
            // not-bound APIs causes module-level ImportError that masks the
            // skip marker.
            if fixtures.iter().all(|f| is_skipped(f, "python")) {
                continue;
            }

            let filename = format!("test_{}.py", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config);

            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "python"
    }
}

// ---------------------------------------------------------------------------
// pyproject.toml
// ---------------------------------------------------------------------------

fn render_pyproject(
    pkg_name: &str,
    _pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let dep_spec = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!(
                "dependencies = [\"{pkg_name}{pkg_version}\", \"pytest>=7.4\", \"pytest-asyncio>=0.23\", \"pytest-timeout>=2.1\"]\n"
            )
        }
        crate::config::DependencyMode::Local => {
            format!(
                "dependencies = [\"{pkg_name}\", \"pytest>=7.4\", \"pytest-asyncio>=0.23\", \"pytest-timeout>=2.1\"]\n\
                 \n\
                 [tool.uv.sources]\n\
                 {pkg_name} = {{ workspace = true }}\n"
            )
        }
    };

    format!(
        r#"[build-system]
build-backend = "setuptools.build_meta"
requires = ["setuptools>=68", "wheel"]

[project]
name = "{pkg_name}-e2e-tests"
version = "0.0.0"
description = "End-to-end tests"
requires-python = ">=3.10"
{dep_spec}
[tool.setuptools]
packages = []

[tool.pytest.ini_options]
asyncio_mode = "auto"
testpaths = ["tests"]
python_files = "test_*.py"
python_functions = "test_*"
addopts = "-v --strict-markers --tb=short"
timeout = 300

[tool.ruff.lint]
ignore = ["PLR2004"]

[tool.ruff.lint.per-file-ignores]
"tests/**" = ["S101", "S108", "PT011", "B017"]
"#
    )
}

// ---------------------------------------------------------------------------
// Config resolution helpers
// ---------------------------------------------------------------------------

fn resolve_function_name(e2e_config: &E2eConfig) -> String {
    resolve_function_name_for_call(&e2e_config.call)
}

fn resolve_function_name_for_call(call_config: &crate::config::CallConfig) -> String {
    call_config
        .overrides
        .get("python")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

fn resolve_module(e2e_config: &E2eConfig) -> String {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.module.clone())
        .unwrap_or_else(|| e2e_config.call.module.replace('-', "_"))
}

fn resolve_options_type(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_type.clone())
}

/// Resolve how json_object args are passed: "kwargs" (default), "dict", or "json".
fn resolve_options_via(e2e_config: &E2eConfig) -> &str {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_via.as_deref())
        .unwrap_or("kwargs")
}

/// Resolve enum field mappings from the Python override config.
fn resolve_enum_fields(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.enum_fields)
        .unwrap_or(&EMPTY)
}

/// Resolve handle nested type mappings from the Python override config.
/// Maps config field names to their Python constructor type names.
fn resolve_handle_nested_types(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_nested_types)
        .unwrap_or(&EMPTY)
}

/// Resolve handle dict type set from the Python override config.
/// Fields in this set use `TypeName({...})` instead of `TypeName(key=val, ...)`.
fn resolve_handle_dict_types(e2e_config: &E2eConfig) -> &std::collections::HashSet<String> {
    static EMPTY: std::sync::LazyLock<std::collections::HashSet<String>> =
        std::sync::LazyLock::new(std::collections::HashSet::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_dict_types)
        .unwrap_or(&EMPTY)
}

fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_conftest(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    let module = resolve_module(e2e_config);
    let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.is_http_test());

    let header = hash::header(CommentStyle::Hash);
    if has_http_fixtures {
        format!(
            r#"{header}"""Pytest configuration for e2e tests."""
import pytest

# Ensure the package is importable.
# The {module} package is expected to be installed in the current environment.


@pytest.fixture
def client(http_test_server):  # noqa: ANN001, ANN201
    """Return a test client bound to the per-test HTTP server."""
    return http_test_server.client()
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

fn render_test_file(category: &str, fixtures: &[&Fixture], e2e_config: &E2eConfig) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "\"\"\"E2e tests for category: {category}.\"\"\"");

    let module = resolve_module(e2e_config);
    let function_name = resolve_function_name(e2e_config);
    let options_type = resolve_options_type(e2e_config);
    let options_via = resolve_options_via(e2e_config);
    let enum_fields = resolve_enum_fields(e2e_config);
    let handle_nested_types = resolve_handle_nested_types(e2e_config);
    let handle_dict_types = resolve_handle_dict_types(e2e_config);
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
    );

    let has_error_test = fixtures
        .iter()
        .any(|f| f.assertions.iter().any(|a| a.assertion_type == "error"));
    let has_skipped = fixtures.iter().any(|f| is_skipped(f, "python"));
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    // Check if any fixture in this file uses an async call.
    let is_async = fixtures.iter().any(|f| {
        let cc = e2e_config.resolve_call(f.call.as_deref());
        cc.r#async
    }) || e2e_config.call.r#async;
    let needs_pytest = has_error_test || has_skipped || is_async;

    // "json" mode needs `import json`.
    let needs_json_import = options_via == "json"
        && fixtures.iter().any(|f| {
            e2e_config
                .call
                .args
                .iter()
                .any(|arg| arg.arg_type == "json_object" && !resolve_field(&f.input, &arg.field).is_null())
        });

    // mock_url args need `import os`.
    let needs_os_import = e2e_config.call.args.iter().any(|arg| arg.arg_type == "mock_url");

    // HTTP tests with header UUID assertions need `import re`.
    let needs_re_import = has_http_tests
        && fixtures.iter().any(|f| {
            f.http
                .as_ref()
                .is_some_and(|h| h.expected_response.headers.values().any(|v| v == "<<uuid>>"))
        });

    // Only import options_type when using "kwargs" mode.
    let needs_options_type = options_via == "kwargs"
        && options_type.is_some()
        && fixtures.iter().any(|f| {
            e2e_config
                .call
                .args
                .iter()
                .any(|arg| arg.arg_type == "json_object" && !resolve_field(&f.input, &arg.field).is_null())
        });

    // Collect enum types actually used across all fixtures in this file.
    let mut used_enum_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if needs_options_type && !enum_fields.is_empty() {
        for fixture in fixtures.iter() {
            for arg in &e2e_config.call.args {
                if arg.arg_type == "json_object" {
                    let value = resolve_field(&fixture.input, &arg.field);
                    if let Some(obj) = value.as_object() {
                        for key in obj.keys() {
                            if let Some(enum_type) = enum_fields.get(key) {
                                used_enum_types.insert(enum_type.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect imports sorted per isort/ruff I001: stdlib group, then
    // third-party group, separated by a blank line. Within each group
    // `import X` lines come before `from X import Y` lines, both sorted.
    let mut stdlib_imports: Vec<String> = Vec::new();
    let mut thirdparty_bare: Vec<String> = Vec::new();
    let mut thirdparty_from: Vec<String> = Vec::new();

    if needs_json_import {
        stdlib_imports.push("import json".to_string());
    }

    if needs_os_import {
        stdlib_imports.push("import os".to_string());
    }

    if needs_re_import {
        stdlib_imports.push("import re".to_string());
    }

    if needs_pytest {
        thirdparty_bare.push("import pytest".to_string());
    }

    // For non-HTTP fixtures, build the normal function imports.
    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test());
    if has_non_http_fixtures {
        // Collect handle constructor function names that need to be imported.
        let handle_constructors: Vec<String> = e2e_config
            .call
            .args
            .iter()
            .filter(|arg| arg.arg_type == "handle")
            .map(|arg| format!("create_{}", arg.name.to_snake_case()))
            .collect();

        // Collect all unique function names actually used across all fixtures in this file.
        // Do not seed with the default function_name — only include it when at least one
        // fixture resolves to it, to avoid unused-import (F401) warnings from ruff.
        let mut import_names: Vec<String> = Vec::new();
        for fixture in fixtures.iter() {
            let cc = e2e_config.resolve_call(fixture.call.as_deref());
            let fn_name = resolve_function_name_for_call(cc);
            if !import_names.contains(&fn_name) {
                import_names.push(fn_name);
            }
        }
        // Safety net: should not occur since the group is non-empty, but ensures
        // import_names is never empty if all fixtures use the default call.
        if import_names.is_empty() {
            import_names.push(function_name.clone());
        }
        for ctor in &handle_constructors {
            if !import_names.contains(ctor) {
                import_names.push(ctor.clone());
            }
        }

        // If any handle arg has config, import the config class (CrawlConfig or options_type).
        let needs_config_import = e2e_config.call.args.iter().any(|arg| {
            arg.arg_type == "handle"
                && fixtures.iter().any(|f| {
                    let val = resolve_field(&f.input, &arg.field);
                    !val.is_null() && val.as_object().is_some_and(|o| !o.is_empty())
                })
        });
        if needs_config_import {
            let config_class = options_type.as_deref().unwrap_or("CrawlConfig");
            if !import_names.contains(&config_class.to_string()) {
                import_names.push(config_class.to_string());
            }
        }

        // Import any nested handle config types actually used in this file.
        if !handle_nested_types.is_empty() {
            let mut used_nested_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for fixture in fixtures.iter() {
                for arg in &e2e_config.call.args {
                    if arg.arg_type == "handle" {
                        let config_value = resolve_field(&fixture.input, &arg.field);
                        if let Some(obj) = config_value.as_object() {
                            for key in obj.keys() {
                                if let Some(type_name) = handle_nested_types.get(key) {
                                    if obj[key].is_object() {
                                        used_nested_types.insert(type_name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for type_name in used_nested_types {
                if !import_names.contains(&type_name) {
                    import_names.push(type_name);
                }
            }
        }

        // Collect method_result helper function imports.
        for fixture in fixtures.iter() {
            for assertion in &fixture.assertions {
                if assertion.assertion_type == "method_result" {
                    if let Some(method_name) = &assertion.method {
                        let import = python_method_helper_import(method_name);
                        if let Some(name) = import {
                            if !import_names.contains(&name) {
                                import_names.push(name);
                            }
                        }
                    }
                }
            }
        }

        if let (true, Some(opts_type)) = (needs_options_type, &options_type) {
            import_names.push(opts_type.clone());
            thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
            // Import enum types from enum_module (if specified) or main module.
            if !used_enum_types.is_empty() {
                let enum_mod = e2e_config
                    .call
                    .overrides
                    .get("python")
                    .and_then(|o| o.enum_module.as_deref())
                    .unwrap_or(&module);
                let enum_names: Vec<&String> = used_enum_types.iter().collect();
                thirdparty_from.push(format!(
                    "from {enum_mod} import {}",
                    enum_names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
        } else {
            thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
        }
    }

    stdlib_imports.sort();
    thirdparty_bare.sort();
    thirdparty_from.sort();

    // Emit sorted import groups with blank lines between groups per PEP 8.
    if !stdlib_imports.is_empty() {
        for imp in &stdlib_imports {
            let _ = writeln!(out, "{imp}");
        }
        let _ = writeln!(out);
    }
    // Third-party: bare imports then from-imports, no blank line between them.
    for imp in &thirdparty_bare {
        let _ = writeln!(out, "{imp}");
    }
    for imp in &thirdparty_from {
        let _ = writeln!(out, "{imp}");
    }
    // Two blank lines after imports (PEP 8 / ruff I001).
    let _ = writeln!(out);
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_function(&mut out, fixture);
        } else {
            render_test_function(
                &mut out,
                fixture,
                e2e_config,
                options_type.as_deref(),
                options_via,
                enum_fields,
                handle_nested_types,
                handle_dict_types,
                &field_resolver,
            );
        }
        let _ = writeln!(out);
    }

    out
}

// ---------------------------------------------------------------------------
// HTTP server test rendering
// ---------------------------------------------------------------------------

/// Render a pytest test function for an HTTP server fixture.
///
/// The generated test:
/// 1. Receives a `client` fixture from conftest.py (the test server client).
/// 2. Sends the configured request.
/// 3. Asserts status code, body (exact or partial), headers, and validation errors.
fn render_http_test_function(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };

    if is_skipped(fixture, "python") {
        let reason = fixture
            .skip
            .as_ref()
            .and_then(|s| s.reason.as_deref())
            .unwrap_or("skipped for python");
        let escaped = escape_python(reason);
        let _ = writeln!(out, "@pytest.mark.skip(reason=\"{escaped}\")");
    }

    let _ = writeln!(out, "def test_{fn_name}(client) -> None:");
    let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");

    // Build the request call.
    let method = http.request.method.to_lowercase();
    let path = &http.request.path;

    // Collect keyword arguments for the request method call.
    let mut call_kwargs: Vec<String> = Vec::new();

    // JSON body
    if let Some(body) = &http.request.body {
        let py_body = json_to_python_literal(body);
        call_kwargs.push(format!("        json={py_body},"));
    }

    // Request headers
    if !http.request.headers.is_empty() {
        let entries: Vec<String> = http
            .request
            .headers
            .iter()
            .map(|(k, v)| format!("            \"{}\": \"{}\",", escape_python(k), escape_python(v)))
            .collect();
        let headers_block = entries.join("\n");
        call_kwargs.push(format!("        headers={{\n{headers_block}\n        }},"));
    }

    // Query params
    if !http.request.query_params.is_empty() {
        let entries: Vec<String> = http
            .request
            .query_params
            .iter()
            .map(|(k, v)| format!("            \"{}\": {},", escape_python(k), json_to_python_literal(v)))
            .collect();
        let params_block = entries.join("\n");
        call_kwargs.push(format!("        params={{\n{params_block}\n        }},"));
    }

    // Cookies
    if !http.request.cookies.is_empty() {
        let entries: Vec<String> = http
            .request
            .cookies
            .iter()
            .map(|(k, v)| format!("            \"{}\": \"{}\",", escape_python(k), escape_python(v)))
            .collect();
        let cookies_block = entries.join("\n");
        call_kwargs.push(format!("        cookies={{\n{cookies_block}\n        }},"));
    }

    if call_kwargs.is_empty() {
        let _ = writeln!(out, "    response = client.{method}(\"{path}\")");
    } else {
        let _ = writeln!(out, "    response = client.{method}(");
        let _ = writeln!(out, "        \"{path}\",");
        for kwarg in &call_kwargs {
            let _ = writeln!(out, "{kwarg}");
        }
        let _ = writeln!(out, "    )");
    }

    // Status code assertion.
    let status = http.expected_response.status_code;
    let _ = writeln!(out, "    assert response.status_code == {status}  # noqa: S101");

    // Body assertions.
    if let Some(expected_body) = &http.expected_response.body {
        let py_val = json_to_python_literal(expected_body);
        let _ = writeln!(out, "    data = response.json()");
        let _ = writeln!(out, "    assert data == {py_val}  # noqa: S101");
    } else if let Some(partial) = &http.expected_response.body_partial {
        let _ = writeln!(out, "    data = response.json()");
        if let Some(obj) = partial.as_object() {
            for (key, val) in obj {
                let py_val = json_to_python_literal(val);
                let escaped_key = escape_python(key);
                let _ = writeln!(out, "    assert data[\"{escaped_key}\"] == {py_val}  # noqa: S101");
            }
        }
    }

    // Header assertions.
    for (header_name, header_value) in &http.expected_response.headers {
        let lower_name = header_name.to_lowercase();
        let escaped_name = escape_python(&lower_name);
        match header_value.as_str() {
            "<<present>>" => {
                let _ = writeln!(out, "    assert \"{escaped_name}\" in response.headers  # noqa: S101");
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    assert response.headers.get(\"{escaped_name}\") is None  # noqa: S101"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    assert re.match(r'^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$', response.headers[\"{escaped_name}\"])  # noqa: S101"
                );
            }
            exact => {
                let escaped_val = escape_python(exact);
                let _ = writeln!(
                    out,
                    "    assert response.headers[\"{escaped_name}\"] == \"{escaped_val}\"  # noqa: S101"
                );
            }
        }
    }

    // Validation error assertions.
    if let Some(validation_errors) = &http.expected_response.validation_errors {
        if !validation_errors.is_empty() {
            let _ = writeln!(out, "    errors = response.json().get(\"detail\", [])");
            for ve in validation_errors {
                let loc_py: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_python(s))).collect();
                let loc_str = loc_py.join(", ");
                let escaped_msg = escape_python(&ve.msg);
                let _ = writeln!(
                    out,
                    "    assert any(e[\"loc\"] == [{loc_str}] and \"{escaped_msg}\" in e[\"msg\"] for e in errors)  # noqa: S101"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &std::collections::HashSet<String>,
    field_resolver: &FieldResolver,
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_function_name_for_call(call_config);
    let result_var = &call_config.result_var;

    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };

    // Emit pytest.mark.skip for fixtures that should be skipped for python.
    if is_skipped(fixture, "python") {
        let reason = fixture
            .skip
            .as_ref()
            .and_then(|s| s.reason.as_deref())
            .unwrap_or("skipped for python");
        let escaped = escape_python(reason);
        let _ = writeln!(out, "@pytest.mark.skip(reason=\"{escaped}\")");
    }

    let is_async = call_config.r#async;
    if is_async {
        let _ = writeln!(out, "@pytest.mark.asyncio");
        let _ = writeln!(out, "async def test_{fn_name}() -> None:");
    } else {
        let _ = writeln!(out, "def test_{fn_name}() -> None:");
    }
    let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");

    // Check if any assertion is an error assertion.
    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Build argument expressions from config.
    let mut arg_bindings = Vec::new();
    let mut kwarg_exprs = Vec::new();
    for arg in &call_config.args {
        let var_name = &arg.name;

        if arg.arg_type == "handle" {
            // Generate a create_engine (or equivalent) call and pass the variable.
            // If there's config data, construct a CrawlConfig with kwargs.
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let config_value = resolve_field(&fixture.input, &arg.field);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                arg_bindings.push(format!("    {var_name} = {constructor_name}(None)"));
            } else if let Some(obj) = config_value.as_object() {
                // Build kwargs for the config constructor (CrawlConfig(key=val, ...)).
                // For fields with a nested type mapping, wrap the dict value in the
                // appropriate typed constructor instead of passing a plain dict.
                let kwargs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let snake_key = k.to_snake_case();
                        let py_val = if let Some(type_name) = handle_nested_types.get(k) {
                            // Wrap the nested dict in the typed constructor.
                            if let Some(nested_obj) = v.as_object() {
                                if nested_obj.is_empty() {
                                    // Empty dict: use the default constructor.
                                    format!("{type_name}()")
                                } else if handle_dict_types.contains(k) {
                                    // The outer Python config type (e.g. CrawlConfig) accepts a
                                    // plain dict for this field (e.g. `auth: dict | None`).
                                    // The binding-layer wrapper (e.g. api.py) creates the typed
                                    // object internally, so we must NOT pre-wrap it here.
                                    json_to_python_literal(v)
                                } else {
                                    // Type takes keyword arguments.
                                    let nested_kwargs: Vec<String> = nested_obj
                                        .iter()
                                        .map(|(nk, nv)| {
                                            let nested_snake_key = nk.to_snake_case();
                                            format!("{nested_snake_key}={}", json_to_python_literal(nv))
                                        })
                                        .collect();
                                    format!("{type_name}({})", nested_kwargs.join(", "))
                                }
                            } else {
                                // Non-object value: use as-is.
                                json_to_python_literal(v)
                            }
                        } else if k == "request_timeout" {
                            // The Python binding converts request_timeout with Duration::from_secs
                            // (seconds) while fixtures specify values in milliseconds. Divide by
                            // 1000 to compensate: e.g., 1 ms → 0 s (immediate timeout),
                            // 5000 ms → 5 s. This keeps test semantics consistent with the
                            // fixture intent.
                            if let Some(ms) = v.as_u64() {
                                format!("{}", ms / 1000)
                            } else {
                                json_to_python_literal(v)
                            }
                        } else {
                            json_to_python_literal(v)
                        };
                        format!("{snake_key}={py_val}")
                    })
                    .collect();
                // Use the options_type if configured, otherwise "CrawlConfig".
                let config_class = options_type.unwrap_or("CrawlConfig");
                let single_line = format!("    {var_name}_config = {config_class}({})", kwargs.join(", "));
                if single_line.len() <= 120 {
                    arg_bindings.push(single_line);
                } else {
                    // Split into multi-line for readability and E501 compliance.
                    let mut lines = format!("    {var_name}_config = {config_class}(\n");
                    for kw in &kwargs {
                        lines.push_str(&format!("        {kw},\n"));
                    }
                    lines.push_str("    )");
                    arg_bindings.push(lines);
                }
                arg_bindings.push(format!("    {var_name} = {constructor_name}({var_name}_config)"));
            } else {
                let literal = json_to_python_literal(config_value);
                arg_bindings.push(format!("    {var_name} = {constructor_name}({literal})"));
            }
            kwarg_exprs.push(format!("{var_name}={var_name}"));
            continue;
        }

        if arg.arg_type == "mock_url" {
            let fixture_id = &fixture.id;
            arg_bindings.push(format!(
                "    {var_name} = os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'"
            ));
            kwarg_exprs.push(format!("{var_name}={var_name}"));
            continue;
        }

        let value = resolve_field(&fixture.input, &arg.field);

        if value.is_null() && arg.optional {
            continue;
        }

        // For json_object args, use the configured options_via strategy.
        if arg.arg_type == "json_object" && !value.is_null() {
            match options_via {
                "dict" => {
                    // Pass as a plain Python dict literal.
                    let literal = json_to_python_literal(value);
                    let noqa = if literal.contains("/tmp/") {
                        "  # noqa: S108"
                    } else {
                        ""
                    };
                    arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
                    kwarg_exprs.push(format!("{var_name}={var_name}"));
                    continue;
                }
                "json" => {
                    // Pass via json.loads() with the raw JSON string.
                    let json_str = serde_json::to_string(value).unwrap_or_default();
                    let escaped = escape_python(&json_str);
                    arg_bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
                    kwarg_exprs.push(format!("{var_name}={var_name}"));
                    continue;
                }
                _ => {
                    // "kwargs" (default): construct OptionsType(key=val, ...).
                    if let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) {
                        let kwargs: Vec<String> = obj
                            .iter()
                            .map(|(k, v)| {
                                let snake_key = k.to_snake_case();
                                let py_val = if let Some(enum_type) = enum_fields.get(k) {
                                    // Map string value to enum constant.
                                    if let Some(s) = v.as_str() {
                                        let upper_val = s.to_shouty_snake_case();
                                        format!("{enum_type}.{upper_val}")
                                    } else {
                                        json_to_python_literal(v)
                                    }
                                } else {
                                    json_to_python_literal(v)
                                };
                                format!("{snake_key}={py_val}")
                            })
                            .collect();
                        let constructor = format!("{opts_type}({})", kwargs.join(", "));
                        arg_bindings.push(format!("    {var_name} = {constructor}"));
                        kwarg_exprs.push(format!("{var_name}={var_name}"));
                        continue;
                    }
                }
            }
        }

        // For required args with no fixture value, use a language-appropriate default.
        if value.is_null() && !arg.optional {
            let default_val = match arg.arg_type.as_str() {
                "string" => "\"\"".to_string(),
                "int" | "integer" => "0".to_string(),
                "float" | "number" => "0.0".to_string(),
                "bool" | "boolean" => "False".to_string(),
                _ => "None".to_string(),
            };
            arg_bindings.push(format!("    {var_name} = {default_val}"));
            kwarg_exprs.push(format!("{var_name}={var_name}"));
            continue;
        }

        let literal = json_to_python_literal(value);
        let noqa = if literal.contains("/tmp/") {
            "  # noqa: S108"
        } else {
            ""
        };
        arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
        kwarg_exprs.push(format!("{var_name}={var_name}"));
    }

    // Generate visitor class if the fixture has a visitor spec.
    if let Some(visitor_spec) = &fixture.visitor {
        let _ = writeln!(out, "    class _TestVisitor:");
        for (method_name, action) in &visitor_spec.callbacks {
            emit_python_visitor_method(out, method_name, action);
        }
        kwarg_exprs.push("visitor=_TestVisitor()".to_string());
    }

    for binding in &arg_bindings {
        let _ = writeln!(out, "{binding}");
    }

    let call_args = kwarg_exprs.join(", ");
    let await_prefix = if is_async { "await " } else { "" };
    let call_expr = format!("{await_prefix}{function_name}({call_args})");

    if has_error_assertion {
        // Find error assertion for optional message check.
        let error_assertion = fixture.assertions.iter().find(|a| a.assertion_type == "error");
        let has_message = error_assertion
            .and_then(|a| a.value.as_ref())
            .and_then(|v| v.as_str())
            .is_some();

        if has_message {
            let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:  # noqa: B017");
            let _ = writeln!(out, "        {call_expr}");
            if let Some(msg) = error_assertion.and_then(|a| a.value.as_ref()).and_then(|v| v.as_str()) {
                let escaped = escape_python(msg);
                let _ = writeln!(out, "    assert \"{escaped}\" in str(exc_info.value)  # noqa: S101");
            }
        } else {
            let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
            let _ = writeln!(out, "        {call_expr}");
        }

        // Skip non-error assertions: `result` is not defined outside the
        // `pytest.raises` block, so referencing it would trigger ruff F821.
        return;
    }

    // Non-error path.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });
    let py_result_var = if has_usable_assertion {
        result_var.to_string()
    } else {
        "_".to_string()
    };
    let _ = writeln!(out, "    {py_result_var} = {call_expr}");

    let fields_enum = &e2e_config.fields_enum;
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" {
            // The call already raises on error in Python.
            continue;
        }
        render_assertion(out, assertion, result_var, field_resolver, fields_enum);
    }
}

// ---------------------------------------------------------------------------
// Argument rendering
// ---------------------------------------------------------------------------

fn json_to_python_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(true) => "True".to_string(),
        serde_json::Value::Bool(false) => "False".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => python_string_literal(s),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_python_literal).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\": {}", escape_python(k), json_to_python_literal(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// Assertion rendering
// ---------------------------------------------------------------------------

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_enum: &std::collections::HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    # skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_access = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "python", result_var),
        _ => result_var.to_string(),
    };

    // Determine whether this field should be compared as an enum string.
    //
    // PyO3 integer-based enums (`#[pyclass(eq, eq_int)]`) are NOT iterable, so
    // `"value" in enum_field` raises TypeError.  Use `str(enum_field).lower()`
    // instead, which for a variant like `LinkType.Anchor` gives `"linktype.anchor"`,
    // making `"anchor" in str(LinkType.Anchor).lower()` evaluate to True.
    //
    // We apply this to fields explicitly listed in `fields_enum` (using both the
    // fixture field path and the resolved path) and to any field whose accessor
    // involves array-element indexing (`[0]`) which typically holds typed enums.
    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        if fields_enum.contains(f) {
            return true;
        }
        let resolved = field_resolver.resolve(f);
        if fields_enum.contains(resolved) {
            return true;
        }
        // Also treat fields accessed via array indexing as potentially enum-typed
        // (e.g., `result.links[0].link_type`, `result.assets[0].asset_category`).
        // This is safe because `str(string_value).lower()` is idempotent for
        // plain string fields, and all fixture `contains` values are lowercase.
        field_resolver.accessor(f, "python", result_var).contains("[0]")
    });

    // Check whether the field path (or any prefix of it) is optional so we can
    // guard `in` / `not in` expressions against None.
    let field_is_optional = match &assertion.field {
        Some(f) if !f.is_empty() => {
            let resolved = field_resolver.resolve(f);
            field_resolver.is_optional(resolved)
        }
        _ => false,
    };

    match assertion.assertion_type.as_str() {
        "error" | "not_error" => {
            // Handled at call site.
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                // Use `is` for boolean/None comparisons (ruff E712).
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                // For string equality, strip trailing whitespace to handle trailing newlines
                // from the converter.
                if val.is_string() {
                    let _ = writeln!(out, "    assert {field_access}.strip() {op} {expected}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {field_access} {op} {expected}  # noqa: S101");
                }
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                // For enum fields, convert to lowercase string for comparison.
                let cmp_expr = if field_is_enum && val.is_string() {
                    format!("str({field_access}).lower()")
                } else {
                    field_access.clone()
                };
                if field_is_optional {
                    let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                    let _ = writeln!(out, "    assert {expected} in {cmp_expr}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {expected} in {cmp_expr}  # noqa: S101");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let expected = value_to_python_string(val);
                    // For enum fields, convert to lowercase string for comparison.
                    let cmp_expr = if field_is_enum && val.is_string() {
                        format!("str({field_access}).lower()")
                    } else {
                        field_access.clone()
                    };
                    if field_is_optional {
                        let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                        let _ = writeln!(out, "    assert {expected} in {cmp_expr}  # noqa: S101");
                    } else {
                        let _ = writeln!(out, "    assert {expected} in {cmp_expr}  # noqa: S101");
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                // For enum fields, convert to lowercase string for comparison.
                let cmp_expr = if field_is_enum && val.is_string() {
                    format!("str({field_access}).lower()")
                } else {
                    field_access.clone()
                };
                if field_is_optional {
                    let _ = writeln!(
                        out,
                        "    assert {field_access} is None or {expected} not in {cmp_expr}  # noqa: S101"
                    );
                } else {
                    let _ = writeln!(out, "    assert {expected} not in {cmp_expr}  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert {field_access}  # noqa: S101");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert not {field_access}  # noqa: S101");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(value_to_python_string).collect();
                let list_str = items.join(", ");
                // For enum fields, convert to lowercase string for comparison.
                let cmp_expr = if field_is_enum {
                    format!("str({field_access}).lower()")
                } else {
                    field_access.clone()
                };
                if field_is_optional {
                    let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                    let _ = writeln!(
                        out,
                        "    assert any(v in {cmp_expr} for v in [{list_str}])  # noqa: S101"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert any(v in {cmp_expr} for v in [{list_str}])  # noqa: S101"
                    );
                }
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} > {expected}  # noqa: S101");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} < {expected}  # noqa: S101");
            }
        }
        "greater_than_or_equal" | "min" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} >= {expected}  # noqa: S101");
            }
        }
        "less_than_or_equal" | "max" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} <= {expected}  # noqa: S101");
            }
        }
        "starts_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access}.startswith({expected})  # noqa: S101");
            }
        }
        "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access}.endswith({expected})  # noqa: S101");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) >= {n}  # noqa: S101");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) <= {n}  # noqa: S101");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) >= {n}  # noqa: S101");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) == {n}  # noqa: S101");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert {field_access} is True  # noqa: S101");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert not {field_access}  # noqa: S101");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_python_method_call(result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out, "    assert {call_expr} is True  # noqa: S101");
                                } else {
                                    let _ = writeln!(out, "    assert {call_expr} is False  # noqa: S101");
                                }
                            } else {
                                let expected = value_to_python_string(val);
                                let _ = writeln!(out, "    assert {call_expr} == {expected}  # noqa: S101");
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    assert {call_expr}  # noqa: S101");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    assert not {call_expr}  # noqa: S101");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "    assert {call_expr} >= {n}  # noqa: S101");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "    assert len({call_expr}) >= {n}  # noqa: S101");
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let expected = value_to_python_string(val);
                            let _ = writeln!(out, "    assert {expected} in {call_expr}  # noqa: S101");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
                        let _ = writeln!(out, "        {call_expr}");
                    }
                    other_check => {
                        panic!("unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    import re  # noqa: PLC0415");
                let _ = writeln!(
                    out,
                    "    assert re.search({expected}, {field_access}) is not None  # noqa: S101"
                );
            }
        }
        other => {
            panic!("unsupported assertion type: {other}");
        }
    }
}

/// Build a Python call expression for a method_result assertion on a tree-sitter Tree.
/// Maps method names to the appropriate Python function calls.
fn build_python_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node().child_count()"),
        "root_node_type" => format!("{result_var}.root_node().kind()"),
        "named_children_count" => format!("{result_var}.root_node().named_child_count()"),
        "has_error_nodes" => format!("tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("tree_error_count({result_var})"),
        "tree_to_sexp" => format!("tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("find_nodes_by_type({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}={}", k, value_to_python_string(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("{result_var}.{method_name}({arg_str})")
            } else {
                format!("{result_var}.{method_name}()")
            }
        }
    }
}

/// Returns the Python import name for a method_result method that uses a
/// module-level helper function (not a method on the result object).
fn python_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("tree_has_error_nodes".to_string()),
        "error_count" | "tree_error_count" => Some("tree_error_count".to_string()),
        "tree_to_sexp" => Some("tree_to_sexp".to_string()),
        "contains_node_type" => Some("tree_contains_node_type".to_string()),
        "find_nodes_by_type" => Some("find_nodes_by_type".to_string()),
        "run_query" => Some("run_query".to_string()),
        // Methods accessed via result_var (e.g. tree.root_node().child_count()) don't need imports.
        _ => None,
    }
}

fn value_to_python_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => python_string_literal(s),
        serde_json::Value::Bool(true) => "True".to_string(),
        serde_json::Value::Bool(false) => "False".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "None".to_string(),
        other => python_string_literal(&other.to_string()),
    }
}

/// Produce a quoted Python string literal, choosing single or double quotes
/// to avoid unnecessary escaping (ruff Q003).
fn python_string_literal(s: &str) -> String {
    if s.contains('"') && !s.contains('\'') {
        // Use single quotes to avoid escaping double quotes.
        let escaped = s
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("'{escaped}'")
    } else {
        format!("\"{}\"", escape_python(s))
    }
}

/// Emit a Python visitor method for a callback action.
fn emit_python_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    let params = match method_name {
        "visit_link" => "self, ctx, href, text, title",
        "visit_image" => "self, ctx, src, alt, title",
        "visit_heading" => "self, ctx, level, text, id",
        "visit_code_block" => "self, ctx, lang, code",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "self, ctx, text",
        "visit_text" => "self, ctx, text",
        "visit_list_item" => "self, ctx, ordered, marker, text",
        "visit_blockquote" => "self, ctx, content, depth",
        "visit_table_row" => "self, ctx, cells, is_header",
        "visit_custom_element" => "self, ctx, tag_name, html",
        "visit_form" => "self, ctx, action_url, method",
        "visit_input" => "self, ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "self, ctx, src",
        "visit_details" => "self, ctx, is_open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "self, ctx, output, *args"
        }
        "visit_list_start" => "self, ctx, ordered, *args",
        "visit_list_end" => "self, ctx, ordered, output, *args",
        _ => "self, ctx, *args",
    };

    let _ = writeln!(
        out,
        "        def {method_name}({params}):  # noqa: A002, ANN001, ANN202, ARG002"
    );
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "            return \"skip\"");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "            return \"continue\"");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "            return \"preserve_html\"");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_python(output);
            let _ = writeln!(out, "            return {{\"custom\": \"{escaped}\"}}");
        }
        CallbackAction::CustomTemplate { template } => {
            // Use single-quoted f-string so that double quotes inside the template
            // (e.g. `QUOTE: "{text}"`) are not misinterpreted as string delimiters.
            // Escape newlines/tabs/backslashes/single quotes so the template stays
            // on a single line in the generated source.
            let escaped_template = template
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            let _ = writeln!(out, "            return {{\"custom\": f'{escaped_template}'}}");
        }
    }
}
