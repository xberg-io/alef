//! Elixir HTTP e2e test rendering.

use crate::e2e::escape::{escape_elixir, sanitize_ident};
use crate::e2e::fixture::{Fixture, HttpFixture, ValidationErrorExpectation};
use std::fmt::Write as _;

use super::values::json_to_elixir;
use crate::e2e::codegen::client;

/// HTTP methods that Finch (Req's underlying HTTP client) does not support.
/// Tests using these methods are emitted with `@tag :skip` so they don't fail.
const FINCH_UNSUPPORTED_METHODS: &[&str] = &["TRACE", "CONNECT"];

/// HTTP methods that Req exposes as convenience functions.
/// All others must be called via `Req.request(method: :METHOD, ...)`.
const REQ_CONVENIENCE_METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head"];

/// Thin renderer that emits ExUnit `describe` + `test` blocks targeting a mock
/// server via `Req`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
pub(super) struct ElixirTestClientRenderer<'a> {
    /// The fixture id is needed in [`render_call`] to build the mock server URL
    /// (`mock_server_url()/fixtures/<id>`).
    fixture_id: &'a str,
    /// Expected response status, needed to disable Req's redirect-following for 3xx.
    expected_status: u16,
}

impl<'a> client::TestClientRenderer for ElixirTestClientRenderer<'a> {
    fn language_name(&self) -> &'static str {
        "elixir"
    }

    /// Emit `describe "{fn_name}" do` + inner `test "METHOD PATH - description" do`.
    ///
    /// When `skip_reason` is `Some`, emit `@tag :skip` before the test block so
    /// ExUnit skips it; the shared driver short-circuits before emitting any
    /// assertions and then calls `render_test_close` for symmetry.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        // ExUnit raises SystemLimitError when a computed test name (its type plus
        // the enclosing describe plus the test name) reaches 255 characters. The
        // computed name here is `test {fn_name} {description}`, so bound the
        // description to keep the whole name under the limit, truncating on a UTF-8
        // char boundary. Each describe wraps a single test, so names stay unique.
        const EXUNIT_NAME_LIMIT: usize = 255;
        // "test " (5) + fn_name + " " (1) + description, with an 8-char safety margin.
        let budget = EXUNIT_NAME_LIMIT.saturating_sub(5 + fn_name.len() + 1 + 8);
        let bounded = if description.len() > budget {
            let mut end = budget;
            while end > 0 && !description.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &description[..end])
        } else {
            description.to_string()
        };
        let escaped_description = bounded.replace('"', "\\\"");
        let _ = writeln!(out, "  describe \"{fn_name}\" do");
        if skip_reason.is_some() {
            let _ = writeln!(out, "    @tag :skip");
        }
        let _ = writeln!(out, "    test \"{escaped_description}\" do");
    }

    /// Close the inner `test` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
    }

    /// Emit a `Req` request to the mock server using `mock_server_url()/fixtures/<id>`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_lowercase();
        // Provide the finch: AlefE2EFinch option so the test uses the named Finch pool started
        // in test_helper.exs instead of Req's default lazy init. The custom Finch pool is
        // configured to use HTTP/1 protocols, so we omit connect_options to avoid Req's
        // "cannot set both :finch and :connect_options" error in 0.5.18+.
        let mut opts: Vec<String> = vec!["finch: AlefE2EFinch".to_string()];

        if let Some(body) = ctx.body {
            let elixir_val = json_to_elixir(body);
            opts.push(format!("json: {elixir_val}"));
        }

        if !ctx.headers.is_empty() {
            let header_pairs: Vec<String> = ctx
                .headers
                .iter()
                .map(|(k, v)| format!("{{\"{}\", \"{}\"}}", escape_elixir(k), escape_elixir(v)))
                .collect();
            opts.push(format!("headers: [{}]", header_pairs.join(", ")));
        }

        if !ctx.cookies.is_empty() {
            let cookie_str = ctx
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            opts.push(format!("headers: [{{\"cookie\", \"{}\"}}]", escape_elixir(&cookie_str)));
        }

        if !ctx.query_params.is_empty() {
            let pairs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    format!("{{\"{}\", \"{}\"}}", escape_elixir(k), escape_elixir(&val_str))
                })
                .collect();
            opts.push(format!("params: [{}]", pairs.join(", ")));
        }

        // When the expected response is a redirect (3xx), disable automatic redirect
        // following so the test can assert the redirect status and Location header.
        if (300..400).contains(&self.expected_status) {
            opts.push("redirect: false".to_string());
        }

        let fixture_id = escape_elixir(self.fixture_id);
        // Use SUT_URL if available (server-pattern), else fall back to mock_server_url() (mock-pattern)
        let sut_url_expr = "System.get_env(\"SUT_URL\") || mock_server_url()";
        let url_expr = format!("({sut_url_expr}) <> \"/fixtures/{fixture_id}\"");

        if REQ_CONVENIENCE_METHODS.contains(&method.as_str()) {
            // `opts` always carries at least the HTTP/1 protocol option.
            let opts_str = opts.join(", ");
            let _ = writeln!(
                out,
                "      {{:ok, response}} = Req.{method}(url: {url_expr}, {opts_str})"
            );
        } else {
            opts.insert(0, format!("method: :{method}"));
            opts.insert(1, format!("url: {url_expr}"));
            let opts_str = opts.join(", ");
            let _ = writeln!(out, "      {{:ok, response}} = Req.request({opts_str})");
        }
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "      assert {response_var}.status == {status}");
    }

    /// Emit a header assertion.
    ///
    /// Handles the special tokens `<<present>>`, `<<absent>>`, `<<uuid>>`.
    /// Skips the `connection` header (hop-by-hop, stripped by Req/Mint).
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        // Req (via Mint) strips hop-by-hop headers; asserting on them is meaningless.
        if header_key == "connection" {
            return;
        }
        let key_lit = format!("\"{}\"", escape_elixir(&header_key));
        let get_header_expr = format!(
            "Enum.find_value({response_var}.headers, fn {{k, v}} -> if String.downcase(k) == {key_lit}, do: List.first(List.wrap(v)) end)"
        );
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "      assert {get_header_expr} != nil");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "      assert {get_header_expr} == nil");
            }
            "<<uuid>>" => {
                let var = sanitize_ident(&header_key);
                let _ = writeln!(out, "      header_val_{var} = {get_header_expr}");
                let _ = writeln!(
                    out,
                    "      assert Regex.match?(~r/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i, to_string(header_val_{var}))"
                );
            }
            literal => {
                let val_lit = format!("\"{}\"", escape_elixir(literal));
                let _ = writeln!(out, "      assert {get_header_expr} == {val_lit}");
            }
        }
    }

    /// Emit a full JSON body equality assertion.
    ///
    /// Req auto-decodes `application/json` bodies; when the response body is a
    /// binary (non-JSON content type), decode it with `Jason.decode!` first.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let elixir_val = json_to_elixir(expected);
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let _ = writeln!(
                    out,
                    "      body_decoded = if is_binary({response_var}.body), do: Jason.decode!({response_var}.body), else: {response_var}.body"
                );
                let _ = writeln!(out, "      assert body_decoded == {elixir_val}");
            }
            _ => {
                let _ = writeln!(out, "      assert {response_var}.body == {elixir_val}");
            }
        }
    }

    /// Emit partial body assertions: one assertion per key in `expected`.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(
                out,
                "      decoded_body = if is_binary({response_var}.body), do: Jason.decode!({response_var}.body), else: {response_var}.body"
            );
            for (key, val) in obj {
                let key_lit = format!("\"{}\"", escape_elixir(key));
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert decoded_body[{key_lit}] == {elixir_val}");
            }
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` appears in
    /// the encoded response body.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_elixir(&err.msg));
            let _ = writeln!(
                out,
                "      assert String.contains?(Jason.encode!({response_var}.body), {msg_lit})"
            );
        }
    }
}

/// Render an ExUnit `describe` + `test` block for an HTTP server test fixture.
///
/// Delegates to [`client::http_call::render_http_test`] after the one
/// Elixir-specific pre-condition: HTTP methods unsupported by Finch (the
/// underlying Req adapter) are emitted with `@tag :skip` directly.
pub(super) fn render_http_test_case(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    let method = http.request.method.to_uppercase();

    // Finch does not support TRACE/CONNECT - emit a skipped test stub directly
    // rather than routing through the shared driver, which would assert on the
    // response and fail.
    if FINCH_UNSUPPORTED_METHODS.contains(&method.as_str()) {
        let test_name = sanitize_ident(&fixture.id);
        let test_label = fixture.id.replace('"', "\\\"");
        let path = &http.request.path;
        let _ = writeln!(out, "  describe \"{test_name}\" do");
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(out, "    test \"{method} {path} - {test_label}\" do");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    let renderer = ElixirTestClientRenderer {
        fixture_id: &fixture.id,
        expected_status: http.expected_response.status_code,
    };
    client::http_call::render_http_test(out, &renderer, fixture);
}
