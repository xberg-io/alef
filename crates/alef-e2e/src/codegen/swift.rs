//! Swift e2e test generator using XCTest.
//!
//! Generates test files for the swift package in `packages/swift/Tests/<Module>Tests/`.
//!
//! IMPORTANT: Due to SwiftPM 6.0 limitations (forbids inter-package `.package(path:)`
//! references within a monorepo), generated test files are placed directly inside
//! the `packages/swift` package (not in a separate `e2e/swift` package). This allows
//! tests to depend on the library target without an explicit package dependency.
//!
//! The generated `Package.swift` is placed in `e2e/swift/` for documentation and CI
//! reference but is NOT used for running tests — tests are run from the
//! `packages/swift/` directory using `swift test`.

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
        _type_defs: &[alef_core::ir::TypeDef],
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
            .or_else(|| config.resolved_version())
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
            &e2e_config.fields_method_calls,
        );

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
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
    // Detect whether any fixture in this group uses a file_path or bytes arg — if so
    // the test class chdir's to <repo>/test_documents at setUp time so the
    // fixture-relative paths in test bodies (e.g. "docx/fake.docx") resolve correctly.
    // The Swift binding's `extractBytes`/`extractFile` e2e wrappers consult
    // `FIXTURES_DIR` first, otherwise resolve against the current directory.
    // Mirrors the Ruby/Python conftest pattern that chdirs to test_documents.
    let needs_chdir = fixtures.iter().any(|f| {
        let call_config = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
        call_config
            .args
            .iter()
            .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
    });

    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import XCTest");
    let _ = writeln!(out, "import Foundation");
    let _ = writeln!(out, "import {module_name}");
    let _ = writeln!(out, "import RustBridge");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// E2e tests for category: {category}.");
    let _ = writeln!(out, "final class {class_name}: XCTestCase {{");

    if needs_chdir {
        // Chdir once at class setUp so all fixture file_path arguments resolve relative
        // to the repository's test_documents directory.
        //
        // #filePath = <repo>/packages/swift/Tests/<Module>Tests/<Class>.swift
        // 5 deletingLastPathComponent() calls climb to the repo root before appending
        // "test_documents". Mirrors the Ruby/Python conftest pattern that chdirs to
        // test_documents.
        let _ = writeln!(out, "    override class func setUp() {{");
        let _ = writeln!(out, "        super.setUp()");
        let _ = writeln!(out, "        let _testDocs = URL(fileURLWithPath: #filePath)");
        let _ = writeln!(out, "            .deletingLastPathComponent() // <Module>Tests/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // Tests/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // swift/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // packages/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // <repo root>");
        let _ = writeln!(
            out,
            "            .appendingPathComponent(\"{}\")",
            e2e_config.test_documents_dir
        );
        let _ = writeln!(
            out,
            "        if FileManager.default.fileExists(atPath: _testDocs.path) {{"
        );
        let _ = writeln!(
            out,
            "            FileManager.default.changeCurrentDirectoryPath(_testDocs.path)"
        );
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
    }

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
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let result_var = &call_config.result_var;
    let args = &call_config.args;
    // Per-call flags override the global default.
    let result_is_simple = call_config.result_is_simple || result_is_simple;
    let result_is_array = call_config.result_is_array;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_config.r#async;

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, &fixture.id, &function_name);

    // Use unqualified function name — the Kreuzberg module (imported by the test)
    // provides convenience overloads that accept plain Swift types (String,
    // [String], JSON strings) and delegate to the RustBridge layer internally.
    let qualified_function_name = function_name.clone();

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
            let _ = writeln!(out, "            _ = try await {qualified_function_name}({args_str})");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        } else {
            let _ = writeln!(
                out,
                "        XCTAssertThrowsError(try {qualified_function_name}({args_str}))"
            );
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    if is_async {
        let _ = writeln!(
            out,
            "        let {result_var} = try await {qualified_function_name}({args_str})"
        );
    } else {
        let _ = writeln!(
            out,
            "        let {result_var} = try {qualified_function_name}({args_str})"
        );
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            result_is_array,
            enum_fields,
        );
    }

    let _ = writeln!(out, "    }}");
}

/// Build setup lines and the argument list for the function call.
///
/// Swift-bridge wrappers require strongly-typed values that don't have implicit
/// Swift literal conversions:
///
/// - `bytes` args become `RustVec<UInt8>` — fixture supplies a relative file path
///   string which is read at test time and pushed into a `RustVec<UInt8>` setup
///   variable. A literal byte array is base64-decoded or UTF-8 encoded inline.
/// - `json_object` args become opaque `ExtractionConfig` (or sibling) instances —
///   a JSON string is decoded via `extractionConfigFromJson(...)` in a setup line.
/// - Optional args missing from the fixture must still appear at the call site
///   as `nil` whenever a later positional arg is present, otherwise Swift slots
///   subsequent values into the wrong parameter.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    fixture_id: &str,
    function_name: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // Pre-compute, for each arg index, whether any later arg has a fixture-provided
    // value (or is required and will emit a default). When an optional arg is empty
    // but a later arg WILL emit, we must keep the slot with `nil` so positional
    // alignment is preserved.
    let later_emits: Vec<bool> = (0..args.len())
        .map(|i| {
            args.iter().skip(i + 1).any(|a| {
                let f = a.field.strip_prefix("input.").unwrap_or(&a.field);
                let v = input.get(f);
                let has_value = matches!(v, Some(x) if !x.is_null());
                has_value || !a.optional || (a.arg_type == "json_object" && a.name == "config")
            })
        })
        .collect();

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "let {} = ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        // bytes args: fixture stores a fixture-relative path string. Generate
        // setup that reads it into a Data and pushes each byte into a
        // RustVec<UInt8>. Literal byte arrays inline the bytes; missing values
        // produce an empty vec (or `nil` when optional).
        if arg.arg_type == "bytes" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push("nil".to_string());
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    let var_name = format!("{}Vec", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {var_name} = RustVec<UInt8>()"));
                    parts.push(var_name);
                }
                Some(serde_json::Value::String(s)) => {
                    let escaped = escape_swift(s);
                    let var_name = format!("{}Vec", arg.name.to_lower_camel_case());
                    let data_var = format!("{}Data", arg.name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {data_var} = try Data(contentsOf: URL(fileURLWithPath: \"{escaped}\"))"
                    ));
                    setup_lines.push(format!("let {var_name} = RustVec<UInt8>()"));
                    setup_lines.push(format!("for _byte in {data_var} {{ {var_name}.push(value: _byte) }}"));
                    parts.push(var_name);
                }
                Some(serde_json::Value::Array(arr)) => {
                    let var_name = format!("{}Vec", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {var_name} = RustVec<UInt8>()"));
                    for v in arr {
                        if let Some(n) = v.as_u64() {
                            setup_lines.push(format!("{var_name}.push(value: UInt8({n}))"));
                        }
                    }
                    parts.push(var_name);
                }
                Some(other) => {
                    // Fallback: encode the JSON serialisation as UTF-8 bytes.
                    let json_str = serde_json::to_string(other).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    let var_name = format!("{}Vec", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {var_name} = RustVec<UInt8>()"));
                    setup_lines.push(format!(
                        "for _byte in Array(\"{escaped}\".utf8) {{ {var_name}.push(value: _byte) }}"
                    ));
                    parts.push(var_name);
                }
            }
            continue;
        }

        // json_object "config" args: the swift-bridge wrapper requires an opaque
        // `ExtractionConfig` (or sibling) instance, not a JSON string. Use the
        // generated `extractionConfigFromJson(_:)` helper from RustBridge.
        // Batch functions (batchExtract*) hardcode config internally — skip it.
        let is_config_arg = arg.name == "config" && arg.arg_type == "json_object";
        let is_batch_fn = function_name.starts_with("batch") || function_name.starts_with("Batch");
        if is_config_arg && !is_batch_fn {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            let json_str = match val {
                None | Some(serde_json::Value::Null) => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            let escaped = escape_swift(&json_str);
            let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
            setup_lines.push(format!("let {var_name} = try extractionConfigFromJson(\"{escaped}\")"));
            parts.push(var_name);
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: keep the slot with `nil`
                // when a later arg will emit, so positional alignment matches
                // the swift-bridge wrapper signature.
                if later_emits[idx] {
                    parts.push("nil".to_string());
                }
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
    result_is_array: bool,
    enum_fields: &HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Skip assertions that traverse a tagged-union variant boundary.
    // In Swift, FormatMetadata and similar enum-backed opaque types are exposed as
    // plain classes by swift-bridge — variant accessor methods (e.g., `.excel()`)
    // are not generated, so such assertions cannot be expressed.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && field_resolver.tagged_union_split(f).is_some() {
            let _ = writeln!(
                out,
                "        // skipped: field '{f}' crosses a tagged-union variant boundary (not expressible in Swift)"
            );
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
    // For other fields: swift-bridge returns all Rust `String` fields as `RustString`.
    // We add .toString() here so string assertions (contains, hasPrefix, etc.) work.
    // Non-string opaque fields (DocumentStructure, etc.) should not appear in string
    // assertions — the fixture schema controls which assertions apply to which fields.
    let string_expr = if field_is_enum {
        format!("{field_expr}.rawValue")
    } else {
        format!("{field_expr}.toString()")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    // For optional strings (String?), use ?? to coalesce before trimming.
                    // `.toString()` converts RustString → Swift String before calling
                    // `.trimmingCharacters`, which requires a concrete String type.
                    let field_is_optional = assertion
                        .field
                        .as_deref()
                        .is_some_and(|f| field_resolver.is_optional(f));
                    let trim_expr = if field_is_optional {
                        format!("(({field_expr})?.toString() ?? \"\").trimmingCharacters(in: .whitespaces)")
                    } else {
                        // string_expr already has .toString() appended; just trim.
                        format!("{string_expr}.trimmingCharacters(in: .whitespaces)")
                    };
                    let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                } else {
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}, {swift_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // When the root result IS the array (result_is_simple + result_is_array) and
                // there is no field path, check array membership via map+contains.
                let no_field = assertion.field.as_deref().is_none_or(|f| f.is_empty());
                if result_is_simple && result_is_array && no_field {
                    // RustVec<RustString> iteration yields RustStringRef (no `toString()`);
                    // use `.as_str().toString()` to convert each element to a Swift String.
                    let _ = writeln!(
                        out,
                        "        XCTAssertTrue({result_var}.map {{ $0.as_str().toString() }}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                    );
                } else {
                    // For array fields (RustVec<RustString>), check membership via map+contains.
                    let field_is_array = assertion
                        .field
                        .as_deref()
                        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
                    if field_is_array {
                        let contains_expr =
                            swift_array_contains_expr(assertion.field.as_deref(), result_var, field_resolver);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue(({contains_expr} ?? []).contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                // For array fields (RustVec<RustString>), check membership via map+contains.
                let field_is_array = assertion
                    .field
                    .as_deref()
                    .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
                if field_is_array {
                    let contains_expr =
                        swift_array_contains_expr(assertion.field.as_deref(), result_var, field_resolver);
                    for val in values {
                        let swift_val = json_to_swift(val);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue(({contains_expr} ?? []).contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                } else {
                    for val in values {
                        let swift_val = json_to_swift(val);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
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
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For string fields, convert to Swift String and check .isEmpty.
            let field_is_optional = assertion
                .field
                .as_deref()
                .is_some_and(|f| field_resolver.is_optional(f));
            if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNotNil({field_expr}, \"expected non-nil value\")");
            } else {
                // string_expr has .toString() appended; .isEmpty works on Swift String.
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.isEmpty, \"expected non-empty value\")"
                );
            }
        }
        "is_empty" => {
            let field_is_optional = assertion
                .field
                .as_deref()
                .is_some_and(|f| field_resolver.is_optional(f));
            if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else {
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.isEmpty, \"expected empty value\")"
                );
            }
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
                // For optional numeric fields, coalesce to 0 before comparing.
                let field_is_optional = assertion
                    .field
                    .as_deref()
                    .is_some_and(|f| field_resolver.is_optional(f));
                let compare_expr = if field_is_optional {
                    format!("({field_expr} ?? 0)")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(out, "        XCTAssertGreaterThan({compare_expr}, {swift_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = assertion
                    .field
                    .as_deref()
                    .is_some_and(|f| field_resolver.is_optional(f));
                let compare_expr = if field_is_optional {
                    format!("({field_expr} ?? 0)")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(out, "        XCTAssertLessThan({compare_expr}, {swift_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields, coalesce to 0 before comparing.
                let field_is_optional = assertion
                    .field
                    .as_deref()
                    .is_some_and(|f| field_resolver.is_optional(f));
                let compare_expr = if field_is_optional {
                    format!("({field_expr} ?? 0)")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({compare_expr}, {swift_val})");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = assertion
                    .field
                    .as_deref()
                    .is_some_and(|f| field_resolver.is_optional(f));
                let compare_expr = if field_is_optional {
                    format!("({field_expr} ?? 0)")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(out, "        XCTAssertLessThanOrEqual({compare_expr}, {swift_val})");
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
                    // Use string_expr.count: for RustString fields string_expr already has
                    // .toString() appended, giving a Swift String whose .count is character count.
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertLessThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // For fields nested inside an optional parent (e.g. document.nodes where
                    // document is Optional), the accessor generates `result.document().nodes()`
                    // which doesn't compile in Swift without optional chaining.
                    let count_expr = swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver);
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let count_expr = swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver);
                    let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
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

/// Build a Swift accessor path for the given fixture field, inserting `()` on
/// every segment and `?` after every optional non-leaf segment.
///
/// This is the core helper for count/contains helpers that need to reconstruct
/// the path with correct optional chaining from the raw fixture field name.
///
/// Returns `(accessor_expr, has_optional)` where `has_optional` is true when
/// at least one `?.` was inserted.
fn swift_build_accessor(field: &str, result_var: &str, field_resolver: &FieldResolver) -> (String, bool) {
    let resolved = field_resolver.resolve(field);
    let parts: Vec<&str> = resolved.split('.').collect();

    // Build a set of optional prefix paths for O(1) lookup during the walk.
    // We track path_so_far incrementally.
    let mut out = result_var.to_string();
    let mut has_optional = false;
    let mut path_so_far = String::new();
    let total = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let is_leaf = i == total - 1;
        if !path_so_far.is_empty() {
            path_so_far.push('.');
        }
        path_so_far.push_str(part);
        out.push('.');
        out.push_str(part);
        out.push_str("()");
        // Insert `?` after `()` for any non-leaf optional field so the next
        // member access becomes `?.`.
        if !is_leaf && field_resolver.is_optional(&path_so_far) {
            out.push('?');
            has_optional = true;
        }
    }
    (out, has_optional)
}

/// Generate a `[String]?` expression for a `RustVec<RustString>` (or optional variant) field
/// so that `contains` membership checks work against plain Swift Strings.
///
/// The result is `Optional<[String]>` — callers should coalesce with `?? []`.
///
/// We use `?.map { $0.as_str().toString() }` because:
/// 1. Iterating a `RustVec<RustString>` yields `RustStringRef` (not `RustString`), which
///    only has `as_str()` but not `toString()` directly.
/// 2. The accessor may end with an `Optional<RustVec<RustString>>` (e.g. `sheet_names()` is
///    `Option<Vec<String>>` in Rust, which becomes `Optional<RustVec<RustString>>` in Swift).
/// 3. Optional chaining from parent `?.` already produces `Optional<RustVec<T>>`.
///
/// `?.map { $0.as_str().toString() }` converts each `RustStringRef` to a Swift `String`,
/// giving `[String]` wrapped in `Optional`. The `?? []` in callers coalesces nil to an empty
/// array.
fn swift_array_contains_expr(field: Option<&str>, result_var: &str, field_resolver: &FieldResolver) -> String {
    let Some(f) = field else {
        return format!("{result_var}.map {{ $0.as_str().toString() }}");
    };
    let (accessor, _has_optional) = swift_build_accessor(f, result_var, field_resolver);
    // Always use `?.map` — the array field (sheet_names, etc.) may itself return
    // Optional<RustVec<T>> even if not listed in fields_optional.
    format!("{accessor}?.map {{ $0.as_str().toString() }}")
}

/// Generate a `.count` expression for an array field that may be nested inside optional parents.
///
/// Swift-bridge exposes all Rust fields as methods with `()`. When ancestor segments are
/// optional, we use `?.` chaining. The final count is coalesced with `?? 0` when there
/// are optional ancestors so the XCTAssert macro receives a non-optional `Int`.
fn swift_array_count_expr(field: Option<&str>, result_var: &str, field_resolver: &FieldResolver) -> String {
    let Some(f) = field else {
        return format!("{result_var}.count");
    };
    let (accessor, has_optional) = swift_build_accessor(f, result_var, field_resolver);
    if has_optional {
        format!("{accessor}.count ?? 0")
    } else {
        format!("{accessor}.count")
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
