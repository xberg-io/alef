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
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
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

        // Resolve client_factory override for swift (enables client-instance dispatch).
        let client_factory: Option<&str> = overrides.and_then(|o| o.client_factory.as_deref());

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
                client_factory,
            );
            files.push(GeneratedFile {
                path: tests_base.join("Tests").join("KreuzbergE2ETests").join(filename),
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
            // SwiftPM 6.0 infers package identity from the path's last component, but the
            // packages/swift/Package.swift declares its name as "Kreuzberg". Use explicit
            // identity specification.
            let dep = format!(r#"        .package(name: "{module_name}", path: "{pkg_path}")"#);
            let prod = format!(r#".product(name: "{module_name}", package: "{module_name}")"#);
            (dep, prod)
        }
    };
    // SwiftPM platform enums use the major version only (.v13, .v14, ...);
    // strip patch components to match the scaffold's `Package.swift`.
    let min_macos_major = min_macos.split('.').next().unwrap_or(min_macos);
    // iOS (.v14) is always included — swift-bridge supports both macOS and iOS targets
    // and the generated Package.swift is used as a CI reference for both platforms.
    format!(
        r#"// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "E2eSwift",
    platforms: [
        .macOS(.v{min_macos_major}),
        .iOS(.v14),
    ],
    dependencies: [
{dep_block},
    ],
    targets: [
        .testTarget(
            name: "KreuzbergE2ETests",
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
    client_factory: Option<&str>,
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
                client_factory,
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
    global_client_factory: Option<&str>,
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    // Per-call client_factory takes precedence over the global one.
    let client_factory: Option<&str> = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or(global_client_factory);
    let result_var = &call_config.result_var;
    let args = &call_config.args;
    // Per-call flags: base call flag OR per-language override OR global flag.
    // Also treat the call as simple when *any* language override marks it as bytes.
    // Calls like `speech()` have `result_is_bytes = true` on C/C#/Java overrides but
    // no explicit `result_is_simple` on the Swift override — yet the Swift binding
    // returns `Data` directly (not a struct), so assertions must use `result.isEmpty`
    // rather than `result.audio().toString().isEmpty`.
    let result_is_bytes_any_lang =
        call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    eprintln!(
        "[swift debug] fixture={} call={:?} result_is_bytes={} any_override_bytes={} overrides={}",
        fixture.id,
        fixture.call,
        call_config.result_is_bytes,
        call_config.overrides.values().any(|o| o.result_is_bytes),
        call_config.overrides.len()
    );
    let result_is_simple = call_config.result_is_simple
        || call_overrides.is_some_and(|o| o.result_is_simple)
        || result_is_simple
        || result_is_bytes_any_lang;
    let result_is_array = call_config.result_is_array;
    // When the call returns `Option<T>` the Swift binding exposes the result as
    // `Optional<…>` (e.g. `getEmbeddingPreset(...) -> EmbeddingPreset?`). Bare-result
    // `is_empty`/`not_empty` assertions must use `XCTAssertNil` / `XCTAssertNotNil`
    // rather than `.toString().isEmpty`, which is undefined on opaque optionals.
    let result_is_option = call_config.result_is_option || call_overrides.is_some_and(|o| o.result_is_option);

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_config.r#async;

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming = crate::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming);
    let collect_snippet_opt = if is_streaming && !expects_error {
        crate::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(lang, result_var, "chunks")
    } else {
        None
    };
    // When swift has streaming-virtual-field assertions but no collect snippet
    // is available (the swift-bridge surface does not yet expose a typed
    // `chatStream` async sequence we can drain into a typed
    // `[ChatCompletionChunk]`), emit a skip stub rather than reference an
    // undefined `chunks` local in the assertion expressions. This keeps the
    // swift test target compiling while the binding catches up.
    if is_streaming && !expects_error && collect_snippet_opt.is_none() {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: streaming chunk collection is not yet supported via the swift-bridge surface (fixture: {})\")",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    let collect_snippet = collect_snippet_opt.unwrap_or_default();

    // Detect whether this call has any json_object args that cannot be constructed
    // in Swift — swift-bridge opaque types do not provide a fromJson initialiser.
    // When such args exist and no `options_via` is configured for swift, emit a
    // skip stub so the test compiles but is recorded as skipped rather than
    // generating invalid code that passes `nil` or a string literal where a
    // strongly-typed request object is required.
    let has_unresolvable_json_object_arg = {
        let options_via = call_overrides.and_then(|o| o.options_via.as_deref());
        options_via.is_none() && args.iter().any(|a| a.arg_type == "json_object" && a.name != "config")
    };

    if has_unresolvable_json_object_arg {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: json_object request construction requires options_via configuration (fixture: {})\");",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    // Resolve extra_args from per-call swift overrides (e.g. `nil` for optional
    // query-param arguments on list_files/list_batches that have no fixture-level
    // input field).
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();

    // Merge per-call enum_fields keys into the effective enum set so that
    // fields like "status" (BatchStatus, BatchObject) are treated as enum-typed
    // even when they are not globally listed in fields_enum (they are context-
    // dependent — BatchStatus on BatchObject but plain String on ResponseObject).
    let effective_enum_fields: std::borrow::Cow<HashSet<String>> = {
        let per_call = call_overrides.map(|o| &o.enum_fields);
        if let Some(pc) = per_call {
            if !pc.is_empty() {
                let mut merged = enum_fields.clone();
                merged.extend(pc.keys().cloned());
                std::borrow::Cow::Owned(merged)
            } else {
                std::borrow::Cow::Borrowed(enum_fields)
            }
        } else {
            std::borrow::Cow::Borrowed(enum_fields)
        }
    };

    let options_via_str: Option<&str> = call_overrides.and_then(|o| o.options_via.as_deref());
    let options_type_str: Option<&str> = call_overrides.and_then(|o| o.options_type.as_deref());
    // Derive the Swift handle-config parsing function from the C override's
    // `c_engine_factory` field. E.g. `"CrawlConfig"` → snake → `"crawl_config_from_json"`
    // → camelCase → `"crawlConfigFromJson"`.
    let handle_config_fn_owned: Option<String> = call_config
        .overrides
        .get("c")
        .and_then(|c| c.c_engine_factory.as_deref())
        .map(|ty| format!("{}_from_json", ty.to_snake_case()).to_lower_camel_case());
    let (setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        fixture.has_host_root_route(),
        &function_name,
        options_via_str,
        options_type_str,
        handle_config_fn_owned.as_deref(),
    );

    // Append extra_args to the argument list.
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // When a client_factory is set, dispatch via a client instance:
    //   let client = try <FactoryType>(apiKey: "test-key", baseUrl: <mock_url>)
    //   try await client.<method>(args)
    // Otherwise fall back to free-function call (Kreuzberg / non-client-factory libraries).
    let has_mock = fixture.mock_response.is_some();
    let (call_setup, call_expr) = if let Some(_factory) = client_factory {
        let env_key = format!("MOCK_SERVER_{}", fixture.id.to_ascii_uppercase().replace('-', "_"));
        let mock_url = if fixture.has_host_root_route() {
            format!(
                "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{}\")",
                fixture.id
            )
        } else {
            format!(
                "ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{}\"",
                fixture.id
            )
        };
        let client_constructor = if has_mock {
            format!("let _client = try DefaultClient(apiKey: \"test-key\", baseUrl: {mock_url})")
        } else {
            // Live API: check for api_key_var; if not present use mock URL anyway.
            if let Some(env_var) = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref()) {
                format!(
                    "let _apiKey = ProcessInfo.processInfo.environment[\"{env_var}\"]\n        \
                     let _baseUrl: String? = _apiKey != nil ? nil : {mock_url}\n        \
                     let _client = try DefaultClient(apiKey: _apiKey ?? \"test-key\", baseUrl: _baseUrl)"
                )
            } else {
                format!("let _client = try DefaultClient(apiKey: \"test-key\", baseUrl: {mock_url})")
            }
        };
        let expr = if is_async {
            format!("try await _client.{function_name}({args_str})")
        } else {
            format!("try _client.{function_name}({args_str})")
        };
        (Some(client_constructor), expr)
    } else {
        // Free-function call (no client_factory).
        let expr = if is_async {
            format!("try await {function_name}({args_str})")
        } else {
            format!("try {function_name}({args_str})")
        };
        (None, expr)
    };
    // For backwards compatibility: qualified_function_name unused when client_factory is set.
    let _ = function_name;

    if is_async {
        let _ = writeln!(out, "    func test{method_name}() async throws {{");
    } else {
        let _ = writeln!(out, "    func test{method_name}() throws {{");
    }
    let _ = writeln!(out, "        // {description}");

    if expects_error {
        // For error fixtures, setup may itself throw (e.g. config validation
        // happens at engine construction). Wrap the whole pipeline — setup
        // and the call — in a single do/catch so any throw counts as success.
        if is_async {
            // XCTAssertThrowsError is a synchronous macro; for async-throwing
            // functions use a do/catch with explicit XCTFail to enforce that
            // the throw actually happens. `await XCTAssertThrowsError(...)` is
            // not valid Swift — it evaluates `await` against a non-async expr.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        } else {
            // Synchronous: emit setup outside (it's expected to succeed) and
            // wrap only the throwing call in XCTAssertThrowsError. If setup
            // itself throws, that propagates as the test's own failure — but
            // sync tests use `throws` so the test method itself rethrows,
            // which XCTest still records as caught. Keep this simple: use a
            // do/catch so setup-time throws also count as expected failures.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    // Emit client construction if a client_factory is configured.
    if let Some(setup) = &call_setup {
        let _ = writeln!(out, "        {setup}");
    }

    let _ = writeln!(out, "        let {result_var} = {call_expr}");

    // Emit the collect snippet for streaming fixtures (drains the async sequence into
    // a local `chunks: [ChatCompletionChunk]` array used by streaming-virtual assertions).
    if !collect_snippet.is_empty() {
        for line in collect_snippet.lines() {
            let _ = writeln!(out, "        {line}");
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            result_is_array,
            result_is_option,
            &effective_enum_fields,
        );
    }

    let _ = writeln!(out, "    }}");
}

#[allow(clippy::too_many_arguments)]
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
    has_host_root_route: bool,
    function_name: &str,
    options_via: Option<&str>,
    options_type: Option<&str>,
    handle_config_fn: Option<&str>,
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
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_ascii_uppercase().replace('-', "_"));
            let url_expr = if has_host_root_route {
                format!(
                    "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("ProcessInfo.processInfo.environment[\"MOCK_SERVER_URL\"]! + \"/fixtures/{fixture_id}\"")
            };
            setup_lines.push(format!("let {} = {url_expr}", arg.name));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_val = input.get(field);
            let has_config = config_val
                .is_some_and(|v| !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty())));
            if has_config {
                if let Some(from_json_fn) = handle_config_fn {
                    let json_str = serde_json::to_string(config_val.unwrap()).unwrap_or_default();
                    let escaped = escape_swift_str(&json_str);
                    let config_var = format!("{}Config", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {config_var} = try {from_json_fn}(\"{escaped}\")"));
                    setup_lines.push(format!("let {var_name} = try createEngine({config_var})"));
                } else {
                    setup_lines.push(format!("let {var_name} = try createEngine(nil)"));
                }
            } else {
                setup_lines.push(format!("let {var_name} = try createEngine(nil)"));
            }
            parts.push(var_name);
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

        // json_object non-config args with options_via = "from_json":
        // Use the generated `{typeCamelCase}FromJson(_:)` helper so the fixture JSON is
        // deserialised into the opaque swift-bridge type rather than passed as a raw string.
        // When arg.field == "input", the entire fixture input IS the request object.
        if arg.arg_type == "json_object" && options_via == Some("from_json") {
            if let Some(type_name) = options_type {
                let resolved_val = super::resolve_field(input, &arg.field);
                let json_str = match resolved_val {
                    serde_json::Value::Null => "{}".to_string(),
                    v => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
                };
                let escaped = escape_swift(&json_str);
                let var_name = format!("_{}", arg.name.to_lower_camel_case());
                let from_json_fn = format!("{}FromJson", type_name.to_lower_camel_case());
                setup_lines.push(format!("let {var_name} = try {from_json_fn}(\"{escaped}\")"));
                parts.push(var_name);
                continue;
            }
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

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    result_is_option: bool,
    enum_fields: &HashSet<String>,
) {
    // When the bare result is `Optional<T>` (no field path) the opaque class
    // exposed by swift-bridge has no `.toString()` method, so the usual
    // `.toString().isEmpty` pattern produces compile errors. Detect the
    // "bare result" case and prefer `XCTAssertNil` / `XCTAssertNotNil`.
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    // Streaming virtual fields resolve against the `chunks` collected-array variable.
    // Intercept before is_valid_for_result so they are never skipped.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && crate::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "swift", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThanOrEqual(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertEqual(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!("        XCTAssertEqual({expr}, \"{escaped}\")\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        XCTAssertEqual({expr}, {b})\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => {
                        format!("        XCTAssertFalse({expr}.isEmpty, \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        XCTAssertTrue({expr}.isEmpty, \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        XCTAssertTrue({expr})\n")
                    }
                    "is_false" => {
                        format!("        XCTAssertFalse({expr})\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThan(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!(
                                "        XCTAssertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    _ => format!(
                        "        // streaming field '{f}': assertion type '{}' not rendered\n",
                        assertion.assertion_type
                    ),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
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

    let field_is_optional = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty() && (field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f)))
    });
    let field_is_array = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty()
            && (field_resolver.is_array(f)
                || field_resolver.is_array(field_resolver.resolve(f))
                || field_resolver.is_collection_root(f)
                || field_resolver.is_collection_root(field_resolver.resolve(f)))
    });

    let field_expr_raw = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "swift", result_var),
            _ => result_var.to_string(),
        }
    };

    // swift-bridge `RustVec<T>` exposes its elements as `T.SelfRef`, which holds
    // a raw pointer into the parent Vec's storage. When the Vec is a temporary
    // (e.g. `result.json_ld()` called inline), Swift ARC may release it before
    // the ref is used, leaving the ref's pointer dangling. Materialise the
    // temporary into a local so it survives the full expression chain.
    //
    // The local name is suffixed with the assertion type plus a hash of the
    // assertion's discriminating fields so multiple assertions on the same
    // collection don't redeclare the same name.
    let local_suffix = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        assertion.field.hash(&mut hasher);
        assertion
            .value
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut hasher);
        format!(
            "{}_{:x}",
            assertion.assertion_type.replace(['-', '.'], "_"),
            hasher.finish() & 0xffff_ffff,
        )
    };
    let (vec_setup, field_expr) = materialise_vec_temporaries(&field_expr_raw, &local_suffix);
    // The `contains` / `not_contains` traversal branch builds its own
    // accessor from `field_resolver.accessor(array_part, ...)`, ignoring
    // `field_expr`. Emitting the vec_setup there would produce dead
    // `let _vec_… = …` lines, so skip it for those traversal cases.
    let field_uses_traversal = assertion.field.as_deref().is_some_and(|f| f.contains("[]."));
    let traversal_skips_field_expr = field_uses_traversal
        && matches!(
            assertion.assertion_type.as_str(),
            "contains" | "not_contains" | "not_empty" | "is_empty"
        );
    if !traversal_skips_field_expr {
        for line in &vec_setup {
            let _ = writeln!(out, "        {line}");
        }
    }

    // In Swift, optional chaining with `?.` makes the result optional even if the
    // called method's return type isn't marked optional. For example:
    // `result.markdown()?.content()` returns `Optional<RustString>` because
    // `markdown()` is optional and the `?.` operator wraps the result.
    // Detect this by checking if the accessor contains `?.`.
    let accessor_is_optional = field_expr.contains("?.");

    // For enum fields, need to handle the string representation differently in Swift.
    // Swift enums don't have `.rawValue` unless they're explicitly RawRepresentable.
    // Check if this is an enum type and handle accordingly.
    // For optional fields (Optional<RustString>), use optional chaining before toString().
    // For other fields: swift-bridge returns all Rust `String` fields as `RustString`.
    // We add .toString() here so string assertions (contains, hasPrefix, etc.) work.
    // Non-string opaque fields (DocumentStructure, etc.) should not appear in string
    // assertions — the fixture schema controls which assertions apply to which fields.
    let string_expr = if field_is_enum && (field_is_optional || accessor_is_optional) {
        // Enum-typed fields that are also optional (e.g. `finish_reason() -> Optional<RustString>`)
        // must use optional chaining: `?.toString() ?? ""` to unwrap before converting to Swift String.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if field_is_enum {
        // Enum-typed fields are now bridged as `String` (RustString in Swift) rather than
        // as opaque enum handles. The getter on the Rust side calls `to_string()` internally
        // and returns a `String` across the FFI. In Swift this arrives as `RustString`, so
        // `.toString()` converts it to a Swift `String` — one call, not two.
        format!("{field_expr}.toString()")
    } else if field_is_optional {
        // Leaf field itself is Optional<RustString> — need ?.toString() to unwrap.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if accessor_is_optional {
        // Ancestor optional chain propagates; leaf is non-optional RustString within chain.
        // Use .toString() directly — the whole expr is Optional<String> due to propagation.
        format!("({field_expr}.toString() ?? \"\")")
    } else {
        format!("{field_expr}.toString()")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    if field_is_enum {
                        // Enum fields: `to_string()` (snake_case) returns RustString;
                        // `.toString()` converts it to a Swift String.
                        // `string_expr` already incorporates this call chain.
                        let trim_expr = format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespaces)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    } else {
                        // For optional strings (String?), use ?? to coalesce before trimming.
                        // `.toString()` converts RustString → Swift String before calling
                        // `.trimmingCharacters`, which requires a concrete String type.
                        // string_expr already incorporates field_is_optional via ?.toString() ?? "".
                        let trim_expr = format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespaces)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    }
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
                    // []. traversal: field like "links[].url" → contains(where:) closure.
                    let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                        if let Some(dot) = f.find("[].") {
                            let array_part = &f[..dot];
                            let elem_part = &f[dot + 3..];
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !traversal_handled {
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
                        } else if field_is_enum {
                            // Enum fields: use `toString().toString()` (via string_expr) to get the
                            // serde variant name as a Swift String, then check substring containment.
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
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
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                // []. traversal: field like "links[].link_type" → contains(where:) per value.
                if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                        }
                        // handled — skip remaining branches
                    } else {
                        // For array fields (RustVec<RustString>), check membership via map+contains.
                        let field_is_array = field_resolver.is_array(field_resolver.resolve(f));
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
                        } else if field_is_enum {
                            // Enum fields: use `toString().toString()` (via string_expr) to get the
                            // serde variant name as a Swift String, then check substring containment.
                            for val in values {
                                let swift_val = json_to_swift(val);
                                let _ = writeln!(
                                    out,
                                    "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
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
                } else {
                    // No field — fall back to existing string_expr path.
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
                // []. traversal: "links[].url" → XCTAssertFalse(array.contains(where:))
                let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        let line = swift_traversal_contains_assert(
                            array_part,
                            elem_part,
                            f,
                            &swift_val,
                            result_var,
                            true,
                            &format!("expected NOT to contain: \\({swift_val})"),
                            enum_fields,
                            field_resolver,
                        );
                        let _ = writeln!(out, "{line}");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !traversal_handled {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                    );
                }
            }
        }
        "not_empty" => {
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For array fields (RustVec<T>), check .isEmpty on the vec directly.
            // For result_is_simple (e.g. Data, String), use .isEmpty directly on
            // the result — avoids calling .toString() on non-RustString types.
            // For string fields, convert to Swift String and check .isEmpty.
            // []. traversal: "links[].url" → contains(where: { !elem.isEmpty })
            let traversal_not_empty_handled = if let Some(f) = assertion.field.as_deref() {
                if let Some(dot) = f.find("[].") {
                    let array_part = &f[..dot];
                    let elem_part = &f[dot + 3..];
                    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
                    let resolved_full = field_resolver.resolve(f);
                    let resolved_elem_part = resolved_full
                        .find("[].")
                        .map(|d| &resolved_full[d + 3..])
                        .unwrap_or(elem_part);
                    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
                    let elem_is_enum = enum_fields.contains(f) || enum_fields.contains(resolved_full);
                    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
                        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
                    let elem_str = if elem_is_enum {
                        format!("{elem_accessor}.to_string().toString()")
                    } else if elem_is_optional {
                        format!("({elem_accessor}?.toString() ?? \"\")")
                    } else {
                        format!("{elem_accessor}.toString()")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertTrue({array_accessor}.contains(where: {{ !{elem_str}.isEmpty }}), \"expected non-empty value\")"
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !traversal_not_empty_handled {
                if bare_result_is_option {
                    let _ = writeln!(out, "        XCTAssertNotNil({result_var}, \"expected non-nil value\")");
                } else if field_is_optional {
                    let _ = writeln!(out, "        XCTAssertNotNil({field_expr}, \"expected non-nil value\")");
                } else if field_is_array {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({field_expr}.isEmpty, \"expected non-empty value\")"
                    );
                } else if result_is_simple {
                    // result_is_simple: result is a primitive (Data, String, etc.) — use .isEmpty directly.
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({result_var}.isEmpty, \"expected non-empty value\")"
                    );
                } else {
                    // Both `RustString` (via RustStringRef.len() -> UInt) and `RustVec<T>` (via
                    // len() -> Int) expose a `.len()` method. Using `.len() > 0` avoids the
                    // `.toString().isEmpty` path that fails to compile when the field returns
                    // `RustVec<T>` — `RustVec<T>` has no `.toString()` member.
                    //
                    // When the accessor contains a `?.` optional chain, `.len()` returns an
                    // Optional (e.g. `UInt?`) which Swift cannot compare directly to `0`;
                    // coalesce via `?? 0` so the assertion typechecks.
                    let len_expr = if accessor_is_optional {
                        format!("({field_expr}.len() ?? 0)")
                    } else {
                        format!("{field_expr}.len()")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertGreaterThan({len_expr}, 0, \"expected non-empty value\")"
                    );
                }
            }
        }
        "is_empty" => {
            if bare_result_is_option {
                let _ = writeln!(out, "        XCTAssertNil({result_var}, \"expected nil value\")");
            } else if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else if field_is_array {
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({field_expr}.isEmpty, \"expected empty value\")"
                );
            } else {
                // Symmetric with not_empty: use .len() == 0 to avoid .toString() on
                // RustVec<T> fields that have no .toString() method. When the accessor
                // contains a `?.` optional chain, coalesce so the comparison typechecks.
                let len_expr = if accessor_is_optional {
                    format!("({field_expr}.len() ?? 0)")
                } else {
                    format!("{field_expr}.len()")
                };
                let _ = writeln!(out, "        XCTAssertEqual({len_expr}, 0, \"expected empty value\")");
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
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
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
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
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
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
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
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
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
/// Rewrite a Swift accessor expression to capture any `RustVec` temporaries
/// in a local before subscripting them. Returns `(setup_lines, rewritten_expr)`.
///
/// swift-bridge's `Vec_<T>$get` returns a raw pointer into the Vec's storage
/// wrapped in a `T.SelfRef`. If the Vec was a temporary, ARC may release it
/// before the ref is dereferenced, leaving the pointer dangling and reads
/// returning empty/garbage. Hoisting the Vec into a `let` binding ties the
/// Vec's lifetime to the enclosing function scope, so the ref stays valid.
///
/// Only the first `()[...]` occurrence per expression is materialised — that
/// covers all current fixture access patterns (single-level subscripts on a
/// result field). Nested subscripts are rare and would need a more elaborate
/// pass; if they appear, this returns conservative output (just the first
/// hoist) which is still correct.
fn materialise_vec_temporaries(expr: &str, name_suffix: &str) -> (Vec<String>, String) {
    let Some(idx) = expr.find("()[") else {
        return (Vec::new(), expr.to_string());
    };
    let after_open = idx + 3; // position after `()[`
    let Some(close_rel) = expr[after_open..].find(']') else {
        return (Vec::new(), expr.to_string());
    };
    let subscript_end = after_open + close_rel; // index of `]`
    let prefix = &expr[..idx + 2]; // includes `()`
    let subscript = &expr[idx + 2..=subscript_end]; // `[N]`
    let tail = &expr[subscript_end + 1..]; // everything after `]`
    let method_dot = expr[..idx].rfind('.').unwrap_or(0);
    let method = &expr[method_dot + 1..idx];
    let local = format!("_vec_{}_{}", method, name_suffix);
    let setup = format!("let {local} = {prefix}");
    let rewritten = format!("{local}{subscript}{tail}");
    (vec![setup], rewritten)
}

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
        // Handle array index subscripts within a segment, e.g. `data[0]`.
        // `data[0]` must become `.data()[0]` not `.data[0]()`.
        // Split at the first `[` if present.
        let (field_name, subscript): (&str, Option<&str>) = if let Some(bracket_pos) = part.find('[') {
            (&part[..bracket_pos], Some(&part[bracket_pos..]))
        } else {
            (part, None)
        };

        if !path_so_far.is_empty() {
            path_so_far.push('.');
        }
        // Build the base path (without subscript) for the optional check. When the
        // segment is e.g. `tool_calls[0]`, we want to check `is_optional` against
        // "choices[0].message.tool_calls" not "choices[0].message.tool_calls[0]".
        let base_path = {
            let mut p = path_so_far.clone();
            p.push_str(field_name);
            p
        };
        // Now push the full part (with subscript if any) so path_so_far is correct
        // for subsequent segment checks.
        path_so_far.push_str(part);

        out.push('.');
        out.push_str(field_name);
        if let Some(sub) = subscript {
            // When the getter for this subscripted field is itself optional
            // (e.g. tool_calls returns Optional<RustVec<T>>), insert `?` before
            // the subscript so Swift unwraps the Optional before indexing.
            let field_is_optional = field_resolver.is_optional(&base_path);
            if field_is_optional {
                out.push_str("()?");
                has_optional = true;
            } else {
                out.push_str("()");
            }
            out.push_str(sub);
            // Do NOT append a trailing `?` after the subscript index: in Swift,
            // `optionalVec?[N]` via `Collection.subscript` returns the element
            // type `T` directly (the subscript is non-optional and the force-unwrap
            // inside RustVec's subscript is unconditional).  Optional chaining
            // already consumed the `?` in `?[N]`, so the result is `T` (non-optional
            // in the compiler's view), and a subsequent `?.member()` would be flagged
            // as "optional chaining on non-optional value".  The parent `has_optional`
            // flag is still set when `field_is_optional` is true, which causes the
            // enclosing expression to be wrapped in `(... ?? fallback)` correctly.
        } else {
            out.push_str("()");
            // Insert `?` after `()` for non-leaf optional fields so the next
            // member access becomes `?.`.
            if !is_leaf && field_resolver.is_optional(&base_path) {
                out.push('?');
                has_optional = true;
            }
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
/// Generate a `XCTAssert{True|False}(array.contains(where: { elem_str.contains(val) }), msg)` line
/// for field paths that traverse a collection with `[].` notation (e.g. `links[].url`).
///
/// `array_part` — left side of `[].` (e.g. `"links"`)
/// `element_part` — right side (e.g. `"url"` or `"link_type"`)
/// `full_field` — original assertion.field (used for enum lookup against the full path)
#[allow(clippy::too_many_arguments)]
fn swift_traversal_contains_assert(
    array_part: &str,
    element_part: &str,
    full_field: &str,
    val_expr: &str,
    result_var: &str,
    negate: bool,
    msg: &str,
    enum_fields: &std::collections::HashSet<String>,
    field_resolver: &FieldResolver,
) -> String {
    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
    let resolved_full = field_resolver.resolve(full_field);
    let resolved_elem_part = resolved_full
        .find("[].")
        .map(|d| &resolved_full[d + 3..])
        .unwrap_or(element_part);
    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
    let elem_is_enum = enum_fields.contains(full_field) || enum_fields.contains(resolved_full);
    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
    let elem_str = if elem_is_enum {
        // Enum-typed fields are bridged as `String` (RustString in Swift).
        // A single `.toString()` converts RustString → Swift String.
        format!("{elem_accessor}.toString()")
    } else if elem_is_optional {
        format!("({elem_accessor}?.toString() ?? \"\")")
    } else {
        format!("{elem_accessor}.toString()")
    };
    let assert_fn = if negate { "XCTAssertFalse" } else { "XCTAssertTrue" };
    format!("        {assert_fn}({array_accessor}.contains(where: {{ {elem_str}.contains({val_expr}) }}), \"{msg}\")")
}

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
///
/// Also check if the field itself (the leaf) is optional, which happens when the field
/// returns Optional<RustVec<T>> (e.g., `links()` may return Optional).
fn swift_array_count_expr(field: Option<&str>, result_var: &str, field_resolver: &FieldResolver) -> String {
    let Some(f) = field else {
        return format!("{result_var}.count");
    };
    let (accessor, mut has_optional) = swift_build_accessor(f, result_var, field_resolver);
    // Also check if the leaf field itself is optional.
    if field_resolver.is_optional(f) {
        has_optional = true;
    }
    if has_optional {
        // In Swift, accessing .count on an optional with ?. returns Optional<Int>,
        // so we coalesce with ?? 0 to get a concrete Int for XCTAssert.
        if accessor.contains("?.") {
            format!("{accessor}.count ?? 0")
        } else {
            // If no ?. but field is optional, the field_expr itself is Optional<RustVec<T>>
            // so we need ?. to call count.
            format!("({accessor}?.count ?? 0)")
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_access::FieldResolver;
    use std::collections::{HashMap, HashSet};

    fn make_resolver_tool_calls() -> FieldResolver {
        // Resolver for `choices[0].message.tool_calls[0].function.name`:
        //   - `choices` is a registered array field
        //   - `choices.message.tool_calls` is optional (Optional<RustVec<ToolCall>>)
        let mut optional = HashSet::new();
        optional.insert("choices.message.tool_calls".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("choices".to_string());
        FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
    }

    /// Regression: after `tool_calls()?[0]` the codegen must NOT append a trailing `?`
    /// before the next segment.  The Swift compiler sees `?[0]` as consuming the optional
    /// chain, yielding `ToolCallRef` (non-optional from the subscript's perspective), so
    /// `?.function()` triggers "cannot use optional chaining on non-optional value".
    ///
    /// The fix: do not emit `?` after the subscript index for non-leaf segments.
    #[test]
    fn optional_vec_subscript_does_not_emit_trailing_question_mark_before_next_segment() {
        let resolver = make_resolver_tool_calls();
        // Access `choices[0].message.tool_calls[0].function.name`:
        //   `tool_calls` is optional, `function` and `name` are non-optional.
        let (accessor, has_optional) =
            swift_build_accessor("choices[0].message.tool_calls[0].function.name", "result", &resolver);
        // `?` before `[0]` is correct (tool_calls is optional).
        // swift_build_accessor uses the raw field name without camelCase conversion.
        assert!(
            accessor.contains("tool_calls()?[0]"),
            "expected `tool_calls()?[0]` for optional tool_calls, got: {accessor}"
        );
        // There must NOT be `?[0]?` (trailing `?` after the index).
        assert!(
            !accessor.contains("?[0]?"),
            "must not emit trailing `?` after subscript index: {accessor}"
        );
        // The expression IS optional overall (tool_calls may be nil).
        assert!(has_optional, "expected has_optional=true for optional field chain");
        // Subsequent member access uses `.` (non-optional chain) not `?.`.
        assert!(
            accessor.contains("[0].function()"),
            "expected `.function()` (non-optional) after subscript: {accessor}"
        );
    }
}
