//! Gleam HTTP test renderer using `gleam_httpc` against `MOCK_SERVER_URL`.

use crate::e2e::codegen::client;
use crate::e2e::escape::{escape_gleam, sanitize_ident};
use crate::e2e::fixture::{Fixture, ValidationErrorExpectation};
use std::fmt::Write as FmtWrite;

/// Gleam HTTP test renderer using `gleam_httpc` against `MOCK_SERVER_URL`.
///
/// Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
pub(super) struct GleamTestClientRenderer;

impl client::TestClientRenderer for GleamTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "gleam"
    }

    /// Gleam identifiers must start with a lowercase letter, not `_` or a digit.
    /// Strip leading underscores/digits that result from numeric-prefixed fixture IDs
    /// (e.g. `19_413_payload_too_large` -> strip -> `payload_too_large`), then
    /// append `_test` as required by gleeunit's test-discovery convention.
    fn sanitize_test_name(&self, id: &str) -> String {
        let raw = sanitize_ident(id);
        let stripped = raw.trim_start_matches(|c: char| c == '_' || c.is_ascii_digit());
        if stripped.is_empty() { raw } else { stripped.to_string() }
    }

    /// Emit `// {description}\npub fn {fn_name}_test() {`.
    ///
    /// gleeunit discovers tests as top-level `pub fn <name>_test()` functions.
    /// Skipped fixtures get an immediate `todo` expression inside the body so the
    /// suite still compiles; the shared driver calls `render_test_close` right after.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "// {description}");
        let _ = writeln!(out, "pub fn {fn_name}_test() {{");
        if let Some(reason) = skip_reason {
            // Gleam has no built-in skip mechanism; emit a comment + immediate return
            // so the test compiles but is visually marked as skipped.
            let escaped = escape_gleam(reason);
            let _ = writeln!(out, "  // skipped: {escaped}");
            let _ = writeln!(out, "  Nil");
        }
    }

    /// Emit the closing `}` for the test function.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    /// Emit a `gleam_httpc` request to `MOCK_SERVER_URL` + `ctx.path`.
    ///
    /// Uses `envoy.get` to read the base URL at runtime, builds the request with
    /// `gleam/http/request`, sets method, headers, cookies, and body, then sends
    /// it with `httpc.send`. The response is bound to `ctx.response_var` (`resp`).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let path = ctx.path;

        let _ = writeln!(out, "  let base_url = case envoy.get(\"MOCK_SERVER_URL\") {{");
        let _ = writeln!(out, "    Ok(u) -> u");
        let _ = writeln!(out, "    Error(_) -> \"http://localhost:8080\"");
        let _ = writeln!(out, "  }}");

        let _ = writeln!(out, "  let assert Ok(req) = request.to(base_url <> \"{path}\")");

        let method_const = match ctx.method.to_uppercase().as_str() {
            "GET" => "Get",
            "POST" => "Post",
            "PUT" => "Put",
            "DELETE" => "Delete",
            "PATCH" => "Patch",
            "HEAD" => "Head",
            "OPTIONS" => "Options",
            _ => "Post",
        };
        let _ = writeln!(out, "  let req = request.set_method(req, http.{method_const})");

        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let escaped_ct = escape_gleam(content_type);
            let _ = writeln!(
                out,
                "  let req = request.set_header(req, \"content-type\", \"{escaped_ct}\")"
            );
        }

        for (name, value) in ctx.headers {
            let lower = name.to_lowercase();
            if matches!(lower.as_str(), "content-length" | "host" | "transfer-encoding") {
                continue;
            }
            let escaped_name = escape_gleam(name);
            let escaped_value = escape_gleam(value);
            let _ = writeln!(
                out,
                "  let req = request.set_header(req, \"{escaped_name}\", \"{escaped_value}\")"
            );
        }

        if !ctx.cookies.is_empty() {
            let cookie_str: Vec<String> = ctx.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let escaped_cookie = escape_gleam(&cookie_str.join("; "));
            let _ = writeln!(
                out,
                "  let req = request.set_header(req, \"cookie\", \"{escaped_cookie}\")"
            );
        }

        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_gleam(&json_str);
            let _ = writeln!(out, "  let req = request.set_body(req, \"{escaped}\")");
        }

        let resp = ctx.response_var;
        let _ = writeln!(out, "  let assert Ok({resp}) = httpc.send(req)");
    }

    /// Emit `resp.status |> should.equal(status)`.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "  {response_var}.status |> should.equal({status})");
    }

    /// Emit a header presence check via `list.find`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_gleam(&name.to_lowercase());
        match expected {
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "  {response_var}.headers\n    |> list.find(fn(h: #(String, String)) {{ h.0 == \"{escaped_name}\" }})\n    |> result.is_ok()\n    |> should.be_false()"
                );
            }
            "<<present>>" | "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "  {response_var}.headers\n    |> list.find(fn(h: #(String, String)) {{ h.0 == \"{escaped_name}\" }})\n    |> result.is_ok()\n    |> should.be_true()"
                );
            }
            literal => {
                let _escaped_value = escape_gleam(literal);
                let _ = writeln!(
                    out,
                    "  {response_var}.headers\n    |> list.find(fn(h: #(String, String)) {{ h.0 == \"{escaped_name}\" }})\n    |> result.is_ok()\n    |> should.be_true()"
                );
            }
        }
    }

    /// Emit `resp.body |> string.trim |> should.equal("...")`.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let escaped = match expected {
            serde_json::Value::String(s) => escape_gleam(s),
            other => escape_gleam(&serde_json::to_string(other).unwrap_or_default()),
        };
        let _ = writeln!(
            out,
            "  {response_var}.body |> string.trim |> should.equal(\"{escaped}\")"
        );
    }

    /// Emit partial body assertions.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let fragment = escape_gleam(&format!("\"{}\":", key));
                let _ = writeln!(
                    out,
                    "  {response_var}.body |> string.contains(\"{fragment}\") |> should.equal(True)"
                );
                let _ = val;
            }
        }
    }

    /// Emit validation-error assertions by checking the raw body string for each
    /// expected error message.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        for err in errors {
            let escaped_msg = escape_gleam(&err.msg);
            let _ = writeln!(
                out,
                "  {response_var}.body |> string.contains(\"{escaped_msg}\") |> should.equal(True)"
            );
        }
    }
}

/// Render an HTTP server test using `gleam_httpc` against `MOCK_SERVER_URL`.
///
/// Delegates to [`client::http_call::render_http_test`] via the shared driver.
pub(super) fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &GleamTestClientRenderer, fixture);
}
