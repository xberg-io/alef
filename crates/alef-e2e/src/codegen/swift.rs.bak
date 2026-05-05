//! Swift e2e test generator using XCTest.
//!
//! Generates `Tests/<Module>Tests/<FixtureId>Tests.swift` files from JSON
//! fixtures (one file per fixture group, mirroring the Kotlin per-test-class
//! style) and a `Package.swift` at the e2e package root.

use crate::config::E2eConfig;
use crate::escape::{escape_java as escape_swift_str, expand_fixture_templates, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup, ValidationErrorExpectation};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::toolchain;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Swift e2e code generator.
pub struct SwiftE2eCodegen;

impl E2eCodegen for SwiftE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let swift_pkg = e2e_config.resolve_package("swift");
        let pkg_name = swift_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_upper_camel_case());
        let pkg_path = swift_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/swift".to_string());
        let pkg_version = swift_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // The Swift module name: UpperCamelCase of the package name.
        let module_name = pkg_name.as_str();

        // Resolve the registry URL: derive from the configured repository when
        // available (with a `.git` suffix per SwiftPM convention). Falls back
        // to a vendor-neutral placeholder when no repo is configured.
        let registry_url = config
            .try_github_repo()
            .map(|repo| {
                let base = repo.trim_end_matches('/').trim_end_matches(".git");
                format!("{base}.git")
            })
            .unwrap_or_else(|_| format!("https://example.invalid/{module_name}.git"));

        // Generate Package.swift (kept for tooling/CI reference but not used
        // for running tests — see note below).
        files.push(GeneratedFile {
            path: output_base.join("Package.swift"),
            content: render_package_swift(module_name, &registry_url, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Swift e2e tests are written into the *packages/swift* package rather
        // than into the separate e2e/swift package.  SwiftPM 6.0 forbids local
        // `.package(path:)` references between packages inside the same git
        // repository, so a standalone e2e/swift package cannot depend on
        // packages/swift.  Placing the test files directly inside
        // packages/swift/Tests/<Module>Tests/ sidesteps the restriction: the
        // tests are part of the same SwiftPM package that defines the library
        // target, so no inter-package dependency is needed.
        //
        // `pkg_path` is expressed relative to the e2e/<lang> directory (e.g.
        // "../../packages/swift").  Joining it onto `output_base` and
        // normalising collapses the traversals to the actual project-root-
        // relative path (e.g. "packages/swift").
        let tests_base = normalize_path(&output_base.join(&pkg_path));

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_name = format!("{}Tests", sanitize_filename(&group.category).to_upper_camel_case());
            let filename = format!("{class_name}.swift");
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                module_name,
                &class_name,
                &function_name,
                result_var,
                &e2e_config.call.args,
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
            );
            files.push(GeneratedFile {
                path: tests_base
                    .join("Tests")
                    .join(format!("{module_name}Tests"))
                    .join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "swift"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_package_swift(
    module_name: &str,
    registry_url: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let min_macos = toolchain::SWIFT_MIN_MACOS;

    // For local deps SwiftPM identity = last path component (e.g. "../../packages/swift" → "swift").
    // For registry deps identity is inferred from the URL.
    // Use explicit .product(name:package:) to avoid ambiguity under tools-version 6.0.
    let (dep_block, product_dep) = match dep_mode {
        crate::config::DependencyMode::Registry => {
            let dep = format!(r#"        .package(url: "{registry_url}", from: "{pkg_version}")"#);
            let pkg_id = registry_url
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .split('/')
                .next_back()
                .unwrap_or(module_name);
            let prod = format!(r#".product(name: "{module_name}", package: "{pkg_id}")"#);
            (dep, prod)
        }
        crate::config::DependencyMode::Local => {
            let dep = format!(r#"        .package(path: "{pkg_path}")"#);
            let pkg_id = pkg_path
                .trim_end_matches('/')
                .split('/')
                .next_back()
                .unwrap_or(module_name);
            let prod = format!(r#".product(name: "{module_name}", package: "{pkg_id}")"#);
            (dep, prod)
        }
    };
    // SwiftPM platform enums use the major version only (.v13, .v14, ...);
    // strip patch components to match the scaffold's `Package.swift`.
    let min_macos_major = min_macos.split('.').next().unwrap_or(min_macos);
    format!(
        r#"// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "E2eSwift",
    platforms: [
        .macOS(.v{min_macos_major}),
    ],
    dependencies: [
{dep_block},
    ],
    targets: [
        .testTarget(
            name: "{module_name}Tests",
            dependencies: [{product_dep}]
        ),
    ]
)
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_name: &str,
    class_name: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import XCTest");
    let _ = writeln!(out, "import {module_name}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// E2e tests for category: {category}.");
    let _ = writeln!(out, "final class {class_name}: XCTestCase {{");

    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_method(&mut out, fixture);
        } else {
            render_test_method(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                field_resolver,
                result_is_simple,
                enum_fields,
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}

// ---------------------------------------------------------------------------
// HTTP test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits XCTest `func test...() throws` methods using `URLSession`
/// against the mock server (`ProcessInfo.processInfo.environment["MOCK_SERVER_URL"]`).
struct SwiftTestClientRenderer;

impl client::TestClientRenderer for SwiftTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "swift"
    }

    fn sanitize_test_name(&self, id: &str) -> String {
        // Swift test methods are `func testFoo()` — upper-camel-case after "test".
        sanitize_ident(id).to_upper_camel_case()
    }

    /// Emit `func test{FnName}() throws {` (or a skip stub when the fixture is skipped).
    ///
    /// XCTest has no first-class skip annotation prior to Swift Testing (`@Test`).
    /// For skipped fixtures we emit `try XCTSkipIf(true, reason)` inside the
    /// function body so XCTest records them as skipped rather than omitting them.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "    /// {description}");
        let _ = writeln!(out, "    func test{fn_name}() throws {{");
        if let Some(reason) = skip_reason {
            let escaped = escape_swift(reason);
            let _ = writeln!(out, "        try XCTSkipIf(true, \"{escaped}\")");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
    }

    /// Emit a synchronous `URLSession` round-trip to the mock server.
    ///
    /// `ProcessInfo.processInfo.environment["MOCK_SERVER_URL"]!` provides the base
    /// URL; the fixture path is appended directly.  The call uses a semaphore so the
    /// generated test body stays synchronous (compatible with `throws` functions —
    /// no `async` XCTest support needed).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_path = escape_swift(ctx.path);

        let _ = writeln!(
            out,
            "        let _baseURL = ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]!"
        );
        let _ = writeln!(
            out,
            "        var _req = URLRequest(url: URL(string: _baseURL + \"{fixture_path}\")!)"
        );
        let _ = writeln!(out, "        _req.httpMethod = \"{method}\"");

        // Headers
        let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
        header_pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in &header_pairs {
            let expanded_v = expand_fixture_templates(v);
            let ek = escape_swift(k);
            let ev = escape_swift(&expanded_v);
            let _ = writeln!(out, "        _req.setValue(\"{ev}\", forHTTPHeaderField: \"{ek}\")");
        }

        // Body
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped_body = escape_swift(&json_str);
            let _ = writeln!(out, "        _req.httpBody = \"{escaped_body}\".data(using: .utf8)");
            let _ = writeln!(
                out,
                "        _req.setValue(\"application/json\", forHTTPHeaderField: \"Content-Type\")"
            );
        }

        let _ = writeln!(out, "        var {}: HTTPURLResponse?", ctx.response_var);
        let _ = writeln!(out, "        var _responseData: Data?");
        let _ = writeln!(out, "        let _sema = DispatchSemaphore(value: 0)");
        let _ = writeln!(
            out,
            "        URLSession.shared.dataTask(with: _req) {{ data, resp, _ in"
        );
        let _ = writeln!(out, "            {} = resp as? HTTPURLResponse", ctx.response_var);
        let _ = writeln!(out, "            _responseData = data");
        let _ = writeln!(out, "            _sema.signal()");
        let _ = writeln!(out, "        }}.resume()");
        let _ = writeln!(out, "        _sema.wait()");
        let _ = writeln!(out, "        let _resp = try XCTUnwrap({})", ctx.response_var);
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(out, "        XCTAssertEqual(_resp.statusCode, {status})");
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let lower_name = name.to_lowercase();
        let header_expr = format!("_resp.value(forHTTPHeaderField: \"{}\")", escape_swift(&lower_name));
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "        XCTAssertNotNil({header_expr})");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "        XCTAssertNil({header_expr})");
            }
            "<<uuid>>" => {
                let _ = writeln!(out, "        let _hdrVal_{lower_name} = try XCTUnwrap({header_expr})");
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil(_hdrVal_{lower_name}.range(of: #\"^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$\"#, options: .regularExpression))"
                );
            }
            exact => {
                let escaped = escape_swift(exact);
                let _ = writeln!(out, "        XCTAssertEqual({header_expr}, \"{escaped}\")");
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let serde_json::Value::String(s) = expected {
            let escaped = escape_swift(s);
            let _ = writeln!(
                out,
                "        let _bodyStr = String(data: try XCTUnwrap(_responseData), encoding: .utf8) ?? \"\""
            );
            let _ = writeln!(
                out,
                "        XCTAssertEqual(_bodyStr.trimmingCharacters(in: .whitespacesAndNewlines), \"{escaped}\")"
            );
        } else {
            let json_str = serde_json::to_string(expected).unwrap_or_default();
            let escaped = escape_swift(&json_str);
            let _ = writeln!(out, "        let _bodyData = try XCTUnwrap(_responseData)");
            let _ = writeln!(
                out,
                "        let _expected = try JSONSerialization.jsonObject(with: \"{escaped}\".data(using: .utf8)!)"
            );
            let _ = writeln!(
                out,
                "        let _actual = try JSONSerialization.jsonObject(with: _bodyData)"
            );
            let _ = writeln!(
                out,
                "        XCTAssertEqual(NSDictionary(dictionary: _expected as? [String: AnyHashable] ?? [:]), NSDictionary(dictionary: _actual as? [String: AnyHashable] ?? [:]))"
            );
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "        let _bodyData = try XCTUnwrap(_responseData)");
            let _ = writeln!(
                out,
                "        let _bodyObj = try XCTUnwrap(try JSONSerialization.jsonObject(with: _bodyData) as? [String: Any])"
            );
            for (key, val) in obj {
                let escaped_key = escape_swift(key);
                let swift_val = json_to_swift(val);
                let _ = writeln!(
                    out,
                    "        XCTAssertEqual(_bodyObj[\"{escaped_key}\"] as? AnyHashable, ({swift_val}) as AnyHashable)"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "        let _bodyData = try XCTUnwrap(_responseData)");
        let _ = writeln!(
            out,
            "        let _bodyObj = try XCTUnwrap(try JSONSerialization.jsonObject(with: _bodyData) as? [String: Any])"
        );
        let _ = writeln!(
            out,
            "        let _errors = _bodyObj[\"errors\"] as? [[String: Any]] ?? []"
        );
        for ve in errors {
            let escaped_msg = escape_swift(&ve.msg);
            let _ = writeln!(
                out,
                "        XCTAssertTrue(_errors.contains(where: {{ ($0[\"msg\"] as? String)?.contains(\"{escaped_msg}\") == true }}), \"expected validation error: {escaped_msg}\")"
            );
        }
    }
}

/// Render an XCTest method for an HTTP server fixture via the shared driver.
///
/// HTTP 101 (WebSocket upgrade) is emitted as a skip stub because `URLSession`
/// cannot handle Upgrade responses.
fn render_http_test_method(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    // HTTP 101 (WebSocket upgrade) — URLSession cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_ident(&fixture.id).to_upper_camel_case();
        let description = fixture.description.replace('"', "\\\"");
        let _ = writeln!(out, "    /// {description}");
        let _ = writeln!(out, "    func test{method_name}() throws {{");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"HTTP 101 WebSocket upgrade cannot be tested via URLSession\")"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    client::http_call::render_http_test(out, &SwiftTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_config.r#async;

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, &fixture.id);

    if is_async {
        let _ = writeln!(out, "    func test{method_name}() async throws {{");
    } else {
        let _ = writeln!(out, "    func test{method_name}() throws {{");
    }
    let _ = writeln!(out, "        // {description}");

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    if expects_error {
        if is_async {
            // XCTAssertThrowsError is a synchronous macro; for async-throwing
            // functions use a do/catch with explicit XCTFail to enforce that
            // the throw actually happens. `await XCTAssertThrowsError(...)` is
            // not valid Swift — it evaluates `await` against a non-async expr.
            let _ = writeln!(out, "        do {{");
            let _ = writeln!(out, "            _ = try await {function_name}({args_str})");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        } else {
            let _ = writeln!(out, "        XCTAssertThrowsError(try {function_name}({args_str}))");
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    if is_async {
        let _ = writeln!(out, "        let {result_var} = try await {function_name}({args_str})");
    } else {
        let _ = writeln!(out, "        let {result_var} = try {function_name}({args_str})");
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            enum_fields,
        );
    }

    let _ = writeln!(out, "    }}");
}

/// Build setup lines and the argument list for the function call.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "let {} = ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                parts.push(json_to_swift(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "swift", result_var),
            _ => result_var.to_string(),
        }
    };

    // For enum fields, use .rawValue to get the string value.
    let string_expr = if field_is_enum {
        format!("{field_expr}.rawValue")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "        XCTAssertEqual({string_expr}.trimmingCharacters(in: .whitespaces), {swift_val})"
                    );
                } else {
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}, {swift_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let swift_val = json_to_swift(val);
                    let _ = writeln!(
                        out,
                        "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        XCTAssertFalse({field_expr}.isEmpty, \"expected non-empty value\")"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "        XCTAssertTrue({field_expr}.isEmpty, \"expected empty value\")"
            );
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let swift_val = json_to_swift(v);
                        format!("{string_expr}.contains({swift_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let _ = writeln!(out, "        XCTAssertGreaterThan({field_expr}, {swift_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let _ = writeln!(out, "        XCTAssertLessThan({field_expr}, {swift_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({field_expr}, {swift_val})");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let _ = writeln!(out, "        XCTAssertLessThanOrEqual({field_expr}, {swift_val})");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasPrefix({swift_val}), \"expected to start with: \\({swift_val})\")"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasSuffix({swift_val}), \"expected to end with: \\({swift_val})\")"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({field_expr}.count, {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertLessThanOrEqual({field_expr}.count, {n})");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({field_expr}.count, {n})");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}.count, {n})");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "        XCTAssertTrue({field_expr})");
        }
        "is_false" => {
            let _ = writeln!(out, "        XCTAssertFalse({field_expr})");
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil({string_expr}.range(of: {swift_val}, options: .regularExpression), \"expected value to match regex: \\({swift_val})\")"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            let _ = writeln!(out, "        // method_result assertions not yet implemented for Swift");
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Normalise a path by resolving `..` components without hitting the filesystem.
///
/// This mirrors what `std::fs::canonicalize` does but works on paths that do
/// not yet exist on disk (generated-file paths).  Only `..` traversals are
/// collapsed; `.` components are dropped; nothing else is changed.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut components = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Pop the last pushed component if there is one that isn't
                // already a `..` (avoids over-collapsing `../../foo`).
                if !components.as_os_str().is_empty() {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components
}

/// Convert a `serde_json::Value` to a Swift literal string.
fn json_to_swift(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_swift(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_swift).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_swift(&json_str))
        }
    }
}

/// Escape a string for embedding in a Swift double-quoted string literal.
fn escape_swift(s: &str) -> String {
    escape_swift_str(s)
}
