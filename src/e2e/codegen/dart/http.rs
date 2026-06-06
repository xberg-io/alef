use std::cell::Cell;
use std::fmt::Write as FmtWrite;

use crate::e2e::fixture::{Fixture, HttpFixture, ValidationErrorExpectation};

use super::values::escape_dart;
use crate::e2e::codegen::client;

// ---------------------------------------------------------------------------
// HTTP server test rendering — DartTestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits `package:test` `test(...)` blocks using `dart:io HttpClient`
/// against the mock server (`Platform.environment['MOCK_SERVER_URL']`).
///
/// Skipped tests are emitted as self-contained stubs (complete test block with
/// `markTestSkipped`) entirely inside `render_test_open`. `render_test_close` uses
/// `in_skip` to emit the right closing token: nothing extra for skip stubs (already
/// closed) vs. `})));` for regular tests.
///
/// `is_redirect` must be set to `true` before invoking the shared driver for 3xx
/// fixtures so that `render_call` can inject `ioReq.followRedirects = false` after
/// the `openUrl` call.
pub(super) struct DartTestClientRenderer {
    /// Set to `true` when `render_test_open` is called with a skip reason so that
    /// `render_test_close` can match the opening shape.
    in_skip: Cell<bool>,
    /// Pre-set to `true` by the thin wrapper when the fixture expects a 3xx response.
    /// `render_call` injects `ioReq.followRedirects = false` when this is `true`.
    is_redirect: Cell<bool>,
}

impl DartTestClientRenderer {
    fn new(is_redirect: bool) -> Self {
        Self {
            in_skip: Cell::new(false),
            is_redirect: Cell::new(is_redirect),
        }
    }
}

impl client::TestClientRenderer for DartTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "dart"
    }

    /// Emit the test opening.
    ///
    /// For skipped fixtures: emit the entire self-contained stub (open + body +
    /// close + blank line) and set `in_skip = true` so `render_test_close` is a
    /// no-op.
    ///
    /// For active fixtures: emit `test('desc', () => _serialized(() => _withRetry(() async {`
    /// leaving the block open for the assertion primitives.
    fn render_test_open(&self, out: &mut String, _fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_desc = escape_dart(description);
        if let Some(reason) = skip_reason {
            let escaped_reason = escape_dart(reason);
            let _ = writeln!(out, "  test('{escaped_desc}', () {{");
            let _ = writeln!(out, "    markTestSkipped('{escaped_reason}');");
            let _ = writeln!(out, "  }});");
            let _ = writeln!(out);
            self.in_skip.set(true);
        } else {
            let _ = writeln!(
                out,
                "  test('{escaped_desc}', () => _serialized(() => _withRetry(() async {{"
            );
            self.in_skip.set(false);
        }
    }

    /// Emit the test closing token.
    ///
    /// No-op for skip stubs (the stub was fully closed in `render_test_open`).
    /// Emits `})));` followed by a blank line for regular tests.
    fn render_test_close(&self, out: &mut String) {
        if self.in_skip.get() {
            // Stub was already closed in render_test_open.
            return;
        }
        let _ = writeln!(out, "  }})));");
        let _ = writeln!(out);
    }

    /// Emit the full `dart:io HttpClient` request scaffolding.
    ///
    /// Emits:
    /// - URL construction from `MOCK_SERVER_URL`.
    /// - `_httpClient.openUrl(method, uri)`.
    /// - `followRedirects = false` when `is_redirect` is pre-set on the renderer.
    /// - Content-Type header, request headers, cookies, optional body bytes.
    /// - `ioReq.contentLength` when a body is present (avoids chunked encoding).
    /// - `ioReq.close()` → `ioResp`.
    /// - Response-body drain into `bodyStr` (always emitted, including for 3xx).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        // dart:io restricted headers (handled automatically by the HTTP stack).
        const DART_RESTRICTED_HEADERS: &[&str] = &["content-length", "host", "transfer-encoding"];

        let method = ctx.method.to_uppercase();
        let escaped_method = escape_dart(&method);

        // Fixture path is `/fixtures/<id>` — extract the id portion for URL construction.
        let fixture_path = escape_dart(ctx.path);

        // Determine effective content-type.
        let has_explicit_content_type = ctx.headers.keys().any(|k| k.to_lowercase() == "content-type");
        let effective_content_type = if has_explicit_content_type {
            ctx.headers
                .iter()
                .find(|(k, _)| k.to_lowercase() == "content-type")
                .map(|(_, v)| v.as_str())
                .unwrap_or("application/json")
        } else if ctx.body.is_some() {
            ctx.content_type.unwrap_or("application/json")
        } else {
            ""
        };

        let _ = writeln!(out, "    final baseUrl = _sutUrl();");
        let _ = writeln!(out, "    final uri = Uri.parse('$baseUrl{fixture_path}');");
        let _ = writeln!(
            out,
            "    final ioReq = await _httpClient.openUrl('{escaped_method}', uri);"
        );

        // Use a fresh (non-persistent) connection per request. Dart's HttpClient keeps
        // connections alive and reuses them; when the mock server closes an idle keep-alive
        // socket, the next reused request races into a "Connection reset by peer". Disabling
        // persistence trades a little speed for deterministic, reset-free runs.
        let _ = writeln!(out, "    ioReq.persistentConnection = false;");

        // Disable automatic redirect following for 3xx fixtures so the test can
        // assert on the redirect status code itself.
        if self.is_redirect.get() {
            let _ = writeln!(out, "    ioReq.followRedirects = false;");
        }

        // Set content-type header.
        if !effective_content_type.is_empty() {
            let escaped_ct = escape_dart(effective_content_type);
            let _ = writeln!(out, "    ioReq.headers.set('content-type', '{escaped_ct}');");
        }

        // Set request headers (skip dart:io restricted headers and content-type, already handled).
        let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
        header_pairs.sort_by_key(|(k, _)| k.as_str());
        for (name, value) in &header_pairs {
            if DART_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            if name.to_lowercase() == "content-type" {
                continue; // Already handled above.
            }
            let escaped_name = escape_dart(&name.to_lowercase());
            let escaped_value = escape_dart(value);
            let _ = writeln!(out, "    ioReq.headers.set('{escaped_name}', '{escaped_value}');");
        }

        // Add cookies.
        if !ctx.cookies.is_empty() {
            let mut cookie_pairs: Vec<(&String, &String)> = ctx.cookies.iter().collect();
            cookie_pairs.sort_by_key(|(k, _)| k.as_str());
            let cookie_str: Vec<String> = cookie_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_dart(&cookie_str.join("; "));
            let _ = writeln!(out, "    ioReq.headers.set('cookie', '{cookie_header}');");
        }

        // Write body bytes if present (bypass charset-based encoding issues).
        // Set contentLength explicitly so Dart sends Content-Length rather than
        // chunked Transfer-Encoding — consistent with Python (urllib) and Go (http)
        // which both set Content-Length automatically. Chunked encoding is valid
        // HTTP/1.1 but some server configurations respond with a connection reset.
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_dart(&json_str);
            let _ = writeln!(out, "    final bodyBytes = utf8.encode('{escaped}');");
            let _ = writeln!(out, "    ioReq.contentLength = bodyBytes.length;");
            let _ = writeln!(out, "    ioReq.add(bodyBytes);");
        }

        let _ = writeln!(out, "    final ioResp = await ioReq.close();");
        // Always drain the response body into `bodyStr` so assertion primitives
        // (render_assert_json_body, render_assert_partial_body, etc.) can reference
        // it unconditionally. For 3xx redirect responses with followRedirects=false,
        // the mock server still sends a response body (e.g. `{}`) — draining it is
        // safe and necessary when the fixture has a body assertion.
        let _ = writeln!(out, "    final bodyStr = await ioResp.transform(utf8.decoder).join();");
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "    expect(ioResp.statusCode, equals({status}), reason: 'status code mismatch');"
        );
    }

    /// Emit a single header assertion, handling special tokens `<<present>>`,
    /// `<<absent>>`, and `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_dart(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    expect(ioResp.headers.value('{escaped_name}'), isNotNull, reason: 'header {escaped_name} should be present');"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    expect(ioResp.headers.value('{escaped_name}'), isNull, reason: 'header {escaped_name} should be absent');"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    expect(ioResp.headers.value('{escaped_name}'), matches(RegExp(r'^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$')), reason: 'header {escaped_name} should be a UUID');"
                );
            }
            exact => {
                let escaped_value = escape_dart(exact);
                let _ = writeln!(
                    out,
                    "    expect(ioResp.headers.value('{escaped_name}'), contains('{escaped_value}'), reason: 'header {escaped_name} mismatch');"
                );
            }
        }
    }

    /// Emit an exact-equality body assertion.
    ///
    /// String bodies are compared as decoded text; structured JSON bodies are
    /// compared via `jsonDecode`.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_dart(&json_str);
                let _ = writeln!(out, "    final bodyJson = jsonDecode(bodyStr);");
                let _ = writeln!(out, "    final expectedJson = jsonDecode('{escaped}');");
                let _ = writeln!(
                    out,
                    "    expect(bodyJson, equals(expectedJson), reason: 'body mismatch');"
                );
            }
            serde_json::Value::String(s) => {
                let escaped = escape_dart(s);
                let _ = writeln!(
                    out,
                    "    expect(bodyStr.trim(), equals('{escaped}'), reason: 'body mismatch');"
                );
            }
            other => {
                let escaped = escape_dart(&other.to_string());
                let _ = writeln!(
                    out,
                    "    expect(bodyStr.trim(), equals('{escaped}'), reason: 'body mismatch');"
                );
            }
        }
    }

    /// Emit partial-body assertions — every key in `expected` must match the
    /// corresponding field in the parsed JSON response.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        let _ = writeln!(
            out,
            "    final partialJson = jsonDecode(bodyStr) as Map<String, dynamic>;"
        );
        if let Some(obj) = expected.as_object() {
            for (idx, (key, val)) in obj.iter().enumerate() {
                let escaped_key = escape_dart(key);
                let json_val = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = escape_dart(&json_val);
                // Use an index-based variable name so keys with special characters
                // don't produce invalid Dart identifiers.
                let _ = writeln!(out, "    final _expectedField{idx} = jsonDecode('{escaped_val}');");
                let _ = writeln!(
                    out,
                    "    expect(partialJson['{escaped_key}'], equals(_expectedField{idx}), reason: 'partial body field \\'{escaped_key}\\' mismatch');"
                );
            }
        }
    }

    /// Emit validation-error assertions for 422 responses.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "    final errBody = jsonDecode(bodyStr) as Map<String, dynamic>;");
        let _ = writeln!(out, "    final errList = (errBody['errors'] ?? []) as List<dynamic>;");
        for ve in errors {
            let loc_dart: Vec<String> = ve.loc.iter().map(|s| format!("'{}'", escape_dart(s))).collect();
            let loc_str = loc_dart.join(", ");
            let escaped_msg = escape_dart(&ve.msg);
            let _ = writeln!(
                out,
                "    expect(errList.any((e) => e is Map && (e['loc'] as List?)?.join(',') == [{loc_str}].join(',') && (e['msg'] as String? ?? '').contains('{escaped_msg}')), isTrue, reason: 'validation error not found: {escaped_msg}');"
            );
        }
    }
}

/// Render a `package:test` `test(...)` block for an HTTP server fixture.
///
/// Delegates to the shared [`client::http_call::render_http_test`] driver via
/// [`DartTestClientRenderer`]. HTTP 101 (WebSocket upgrade) fixtures are emitted
/// as skip stubs before reaching the driver because `dart:io HttpClient` cannot
/// handle protocol-switch responses.
pub(super) fn render_http_test_case(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) — dart:io HttpClient cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let description = escape_dart(&fixture.description);
        let _ = writeln!(out, "  test('{description}', () {{");
        let _ = writeln!(
            out,
            "    markTestSkipped('Skipped: Dart HttpClient cannot handle 101 Switching Protocols responses');"
        );
        let _ = writeln!(out, "  }});");
        let _ = writeln!(out);
        return;
    }

    // Pre-set `is_redirect` on the renderer so `render_call` can inject
    // `ioReq.followRedirects = false` for 3xx fixtures. The shared driver has no
    // concept of expected status code so we thread it through renderer state.
    let is_redirect = http.expected_response.status_code / 100 == 3;
    client::http_call::render_http_test(out, &DartTestClientRenderer::new(is_redirect), fixture);
}

/// Infer a MIME type from a file path extension.
///
/// Returns `None` when the extension is unknown so the caller can supply a fallback.
/// Used in dart e2e tests when a fixture omits `mime_type` but uses a `file_path` arg.
pub(super) fn mime_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    match ext.to_lowercase().as_str() {
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "pdf" => Some("application/pdf"),
        "txt" | "text" => Some("text/plain"),
        "html" | "htm" => Some("text/html"),
        "json" => Some("application/json"),
        "xml" => Some("application/xml"),
        "csv" => Some("text/csv"),
        "md" | "markdown" => Some("text/markdown"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "zip" => Some("application/zip"),
        "odt" => Some("application/vnd.oasis.opendocument.text"),
        "ods" => Some("application/vnd.oasis.opendocument.spreadsheet"),
        "odp" => Some("application/vnd.oasis.opendocument.presentation"),
        "rtf" => Some("application/rtf"),
        "epub" => Some("application/epub+zip"),
        "msg" => Some("application/vnd.ms-outlook"),
        "eml" => Some("message/rfc822"),
        // Source-code extensions resolve to the internal `text/x-source-code` MIME.
        // The bytes-path can't extract these (CodeExtractor::extract_bytes needs a
        // shebang for language detection), so the caller code in this module
        // checks the inferred MIME and routes source-code files through
        // `extractFileSync`/`extractFile` (path-based) instead of remapping to
        // the bytes facade.
        "py" | "rs" | "go" | "java" | "kt" | "kts" | "swift" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "rb"
        | "php" | "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "cs" | "scala" | "ex" | "exs" | "erl"
        | "hrl" | "elm" | "ml" | "mli" | "fs" | "fsx" | "hs" | "lhs" | "lua" | "pl" | "pm" | "r" | "R" | "sh"
        | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "psd1" | "dart" | "groovy" | "gd" | "nim" | "zig" | "v"
        | "vhdl" | "sv" | "svh" => Some("text/x-source-code"),
        _ => None,
    }
}
