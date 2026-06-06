use crate::e2e::codegen::client;
use crate::e2e::escape::{expand_fixture_templates, sanitize_ident};
use crate::e2e::fixture::{Fixture, ValidationErrorExpectation};
use heck::ToUpperCamelCase;
use std::fmt::Write as _;

use super::values::{escape_swift, json_to_swift};

// ---------------------------------------------------------------------------
// HTTP test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits XCTest `func test...() throws` methods using `URLSession`
/// against the mock server (`ProcessInfo.processInfo.environment["MOCK_SERVER_URL"]`).
pub(super) struct SwiftTestClientRenderer;

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

    /// Emit a synchronous `URLSession` round-trip to the SUT server.
    ///
    /// `ProcessInfo.processInfo.environment["SUT_URL"]!` provides the base
    /// URL; the fixture path is appended directly.  The call uses a semaphore so the
    /// generated test body stays synchronous (compatible with `throws` functions —
    /// no `async` XCTest support needed).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_path = escape_swift(ctx.path);

        let _ = writeln!(
            out,
            "        let _baseURL = ProcessInfo.processInfo.environment[\"SUT_URL\"] ?? \"http://127.0.0.1:8009\""
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
            "        let _session = URLSession(configuration: .ephemeral, delegate: AlefE2ENoRedirectDelegate(), delegateQueue: nil)"
        );
        let _ = writeln!(out, "        _session.dataTask(with: _req) {{ data, resp, _ in");
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
        // Header names contain characters illegal in Swift identifiers (e.g. the `-` in
        // `x-request-id`), so derive a safe local-variable suffix for any binding we emit.
        let var_suffix: String = lower_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "        XCTAssertNotNil({header_expr})");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "        XCTAssertNil({header_expr})");
            }
            "<<uuid>>" => {
                let _ = writeln!(out, "        let _hdrVal_{var_suffix} = try XCTUnwrap({header_expr})");
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil(_hdrVal_{var_suffix}.range(of: #\"^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$\"#, options: .regularExpression))"
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
            let _ = writeln!(
                out,
                "        let _expected = try JSONSerialization.jsonObject(with: \"{escaped}\".data(using: .utf8)!)"
            );
            // Unwrap the response data inline rather than via a shared `_bodyData` local: a
            // fixture may trigger several body assertions in one test, and a repeated
            // `let _bodyData` would be an invalid redeclaration. The leading `try` covers the
            // nested XCTUnwrap call.
            let _ = writeln!(
                out,
                "        let _actual = try JSONSerialization.jsonObject(with: XCTUnwrap(_responseData))"
            );
            let _ = writeln!(
                out,
                "        XCTAssertEqual(NSDictionary(dictionary: _expected as? [String: AnyHashable] ?? [:]), NSDictionary(dictionary: _actual as? [String: AnyHashable] ?? [:]))"
            );
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(
                out,
                "        let _bodyObj = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(_responseData)) as? [String: Any])"
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
        let _ = writeln!(
            out,
            "        let _errorsBodyObj = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(_responseData)) as? [String: Any])"
        );
        let _ = writeln!(
            out,
            "        let _errors = _errorsBodyObj[\"errors\"] as? [[String: Any]] ?? []"
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
pub(super) fn render_http_test_method(out: &mut String, fixture: &Fixture) {
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
