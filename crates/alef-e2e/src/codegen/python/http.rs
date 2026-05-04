//! HTTP server test function rendering for Python e2e tests.

use std::fmt::Write as FmtWrite;

use crate::escape::{escape_python, sanitize_ident};
use crate::fixture::{Fixture, ValidationErrorExpectation};

use super::super::client;
use super::json::json_to_python_literal;

/// Pytest/urllib test renderer.
///
/// Python HTTP e2e tests use `urllib.request` directly against the mock server
/// binary (not a `TestClient` over FFI). The trait primitives emit the urllib
/// request-build + response-capture scaffolding that the existing monolithic
/// renderer produced, so generated output is unchanged after the migration.
struct PythonTestClientRenderer;

impl client::TestClientRenderer for PythonTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "python"
    }

    /// Emit `@pytest.mark.skip` (if skipped), function signature, and docstring.
    ///
    /// Skipped tests still get a stub body (`...`) so pytest can collect them.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let desc_with_period = if description.ends_with('.') {
            description.to_string()
        } else {
            format!("{description}.")
        };

        if let Some(reason) = skip_reason {
            let escaped = escape_python(reason);
            let _ = writeln!(out, "@pytest.mark.skip(reason=\"{escaped}\")");
        }
        let _ = writeln!(out, "def test_{fn_name}(mock_server: str) -> None:");
        let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");
        if skip_reason.is_some() {
            let _ = writeln!(out, "    ...");
        }
    }

    /// No-op: Python functions are not wrapped in a block, so no closing token
    /// is needed. The blank line between tests is emitted by the call site
    /// (`render_test_file`) after every fixture, which keeps the separator
    /// consistent with non-HTTP fixtures.
    fn render_test_close(&self, _out: &mut String) {}

    /// Emit the urllib request scaffolding that drives the mock server.
    ///
    /// Emits:
    /// - Inline imports for `os`, `urllib.request` (and optionally `json`).
    /// - URL construction from the fixture path.
    /// - Headers dict, optional JSON body, and `urllib.request.Request` build.
    /// - A `_NoRedirect` opener + try/except that captures `status_code`,
    ///   `resp_body`, and `resp_headers`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let _ = writeln!(out, "    import os  # noqa: PLC0415");
        let _ = writeln!(out, "    import urllib.request  # noqa: PLC0415");
        let _ = writeln!(out, "    base = os.environ.get(\"MOCK_SERVER_URL\", mock_server)");
        let _ = writeln!(out, "    url = f\"{{base}}{}\"", ctx.path);

        let method = ctx.method.to_uppercase();

        // Build headers dict literal.
        let mut header_entries: Vec<String> = ctx
            .headers
            .iter()
            .map(|(k, v)| format!("        \"{}\": \"{}\",", escape_python(k), escape_python(v)))
            .collect();
        header_entries.sort(); // deterministic output
        let headers_py = if header_entries.is_empty() {
            "{}".to_string()
        } else {
            format!("{{\n{}\n    }}", header_entries.join("\n"))
        };

        if let Some(body) = ctx.body {
            let py_body = json_to_python_literal(body);
            let _ = writeln!(out, "    import json  # noqa: PLC0415");
            let _ = writeln!(out, "    _headers = {headers_py}");
            let _ = writeln!(out, "    _headers.setdefault(\"Content-Type\", \"application/json\")");
            let _ = writeln!(out, "    _body = json.dumps({py_body}).encode()");
            let _ = writeln!(
                out,
                "    _req = urllib.request.Request(url, data=_body, headers=_headers, method=\"{method}\")"
            );
        } else {
            let _ = writeln!(out, "    _headers = {headers_py}");
            let _ = writeln!(
                out,
                "    _req = urllib.request.Request(url, headers=_headers, method=\"{method}\")"
            );
        }

        // Build a no-redirect opener and capture the response.
        // Both `resp_body` and `resp_headers` are always bound so that
        // `render_assert_*` primitives can reference them unconditionally.
        let _ = writeln!(
            out,
            "    class _NoRedirect(urllib.request.HTTPRedirectHandler):  # noqa: N801"
        );
        let _ = writeln!(
            out,
            "        def redirect_request(self, *args, **kwargs): return None  # noqa: E704"
        );
        let _ = writeln!(out, "    _opener = urllib.request.build_opener(_NoRedirect())");
        let _ = writeln!(out, "    try:");
        let _ = writeln!(out, "        response = _opener.open(_req)  # noqa: S310");
        let _ = writeln!(out, "        status_code = response.status");
        let _ = writeln!(out, "        resp_body = response.read()  # noqa: F841");
        let _ = writeln!(out, "        resp_headers = dict(response.headers)  # noqa: F841");
        let _ = writeln!(out, "    except urllib.error.HTTPError as _exc:");
        let _ = writeln!(out, "        status_code = _exc.code");
        let _ = writeln!(out, "        resp_body = _exc.read()  # noqa: F841");
        let _ = writeln!(out, "        resp_headers = dict(_exc.headers)  # noqa: F841");
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(out, "    assert status_code == {status}  # noqa: S101");
    }

    /// Emit a single header assertion, handling special tokens `<<present>>`,
    /// `<<absent>>`, and `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_python(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "    assert \"{escaped_name}\" in resp_headers  # noqa: S101");
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    assert resp_headers.get(\"{escaped_name}\") is None  # noqa: S101"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(out, "    import re  # noqa: PLC0415");
                let _ = writeln!(
                    out,
                    "    assert re.match(r'^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$', resp_headers[\"{escaped_name}\"])  # noqa: S101"
                );
            }
            exact => {
                let escaped_val = escape_python(exact);
                let _ = writeln!(
                    out,
                    "    assert resp_headers[\"{escaped_name}\"] == \"{escaped_val}\"  # noqa: S101"
                );
            }
        }
    }

    /// Emit an exact-equality body assertion.
    ///
    /// String bodies are compared as decoded text; structured JSON bodies are
    /// compared via `json.loads()`.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let serde_json::Value::String(s) = expected {
            let py_val = format!("\"{}\"", escape_python(s));
            let _ = writeln!(out, "    assert resp_body.decode() == {py_val}  # noqa: S101");
        } else {
            let py_val = json_to_python_literal(expected);
            let _ = writeln!(out, "    import json as _json  # noqa: PLC0415");
            let _ = writeln!(out, "    data = _json.loads(resp_body)");
            let _ = writeln!(out, "    assert data == {py_val}  # noqa: S101");
        }
    }

    /// Emit partial-body assertions — every key in `expected` must match the
    /// corresponding value in the parsed JSON response.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        let _ = writeln!(out, "    import json as _json  # noqa: PLC0415");
        let _ = writeln!(out, "    data = _json.loads(resp_body)");
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let py_val = json_to_python_literal(val);
                let escaped_key = escape_python(key);
                let _ = writeln!(out, "    assert data[\"{escaped_key}\"] == {py_val}  # noqa: S101");
            }
        }
    }

    /// Emit validation-error assertions for 422 responses.
    ///
    /// The driver only calls this when `body` is absent (fixture has no exact
    /// body assertion) — if a full body assertion was already emitted the driver
    /// skips validation errors because the body already covers them.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "    import json as _json  # noqa: PLC0415");
        let _ = writeln!(out, "    _data = _json.loads(resp_body)");
        let _ = writeln!(out, "    errors = _data.get(\"errors\", [])");
        for ve in errors {
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

/// Render a pytest test function for an HTTP server fixture.
///
/// Delegates to [`client::http_call::render_http_test`] via [`PythonTestClientRenderer`].
/// HTTP 101 (WebSocket upgrade) is handled as a pre-hook: urllib cannot drive
/// upgrade responses, so those fixtures are emitted as skip-stubs before the
/// shared driver is invoked.
pub(super) fn render_http_test_function(out: &mut String, fixture: &Fixture) {
    // HTTP 101 (WebSocket upgrade) — urllib cannot handle upgrade responses.
    // Emit a skip stub independently of the shared driver.
    if let Some(http) = &fixture.http {
        if http.expected_response.status_code == 101 {
            let fn_name = sanitize_ident(&fixture.id);
            let description = &fixture.description;
            let desc_with_period = if description.ends_with('.') {
                description.to_string()
            } else {
                format!("{description}.")
            };
            let _ = writeln!(
                out,
                "@pytest.mark.skip(reason=\"HTTP 101 WebSocket upgrade cannot be tested via urllib\")"
            );
            let _ = writeln!(out, "def test_{fn_name}(mock_server: str) -> None:");
            let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");
            let _ = writeln!(out, "    ...");
            let _ = writeln!(out);
            return;
        }
    }

    client::http_call::render_http_test(out, &PythonTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_http_test_function_no_http_field_emits_nothing() {
        let fixture = crate::fixture::Fixture {
            id: "test_fixture".to_string(),
            description: "A test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            visitor: None,
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture);
        assert!(out.is_empty(), "got: {out}");
    }
}
