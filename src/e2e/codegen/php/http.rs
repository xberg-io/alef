//! PHP HTTP-specific e2e rendering.

use crate::e2e::escape::{escape_php, sanitize_filename};
use crate::e2e::fixture::{Fixture, HttpFixture, ValidationErrorExpectation};

use crate::e2e::codegen::client;

// ---------------------------------------------------------------------------
// HTTP test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Thin renderer that emits PHPUnit test methods targeting a mock server via
/// Guzzle. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
pub(super) struct PhpTestClientRenderer;

impl client::TestClientRenderer for PhpTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "php"
    }

    /// Convert a fixture id to a PHP-valid identifier (snake_case via `sanitize_filename`).
    fn sanitize_test_name(&self, id: &str) -> String {
        sanitize_filename(id)
    }

    /// Emit `/** {description} */ public function test_{fn_name}(): void {`.
    ///
    /// When `skip_reason` is `Some`, emits a `markTestSkipped(...)` body and the
    /// shared driver calls `render_test_close` immediately after, so the closing
    /// brace is emitted symmetrically.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_reason = skip_reason.map(escape_php);
        let rendered = crate::e2e::template_env::render(
            "php/http_test_open.jinja",
            minijinja::context! {
                fn_name => fn_name,
                description => description,
                skip_reason => escaped_reason,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit the closing `}` for a test method.
    fn render_test_close(&self, out: &mut String) {
        let rendered = crate::e2e::template_env::render("php/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit a Guzzle request to the mock server's `/fixtures/<fixture_id>` endpoint.
    ///
    /// The fixture id is extracted from the path (which the mock server routes as
    /// `/fixtures/<id>`). `$response` is bound for subsequent assertion methods.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();

        // Build Guzzle options array.
        let mut opts: Vec<String> = Vec::new();

        if let Some(body) = ctx.body {
            let php_body = super::values::json_to_php(body);
            opts.push(format!("'json' => {php_body}"));
        }

        // Merge explicit headers and content_type hint.
        let mut header_pairs: Vec<String> = Vec::new();
        if let Some(ct) = ctx.content_type {
            // Only emit if not already in ctx.headers (avoid duplicate Content-Type).
            if !ctx.headers.keys().any(|k| k.to_lowercase() == "content-type") {
                header_pairs.push(format!("\"Content-Type\" => \"{}\"", escape_php(ct)));
            }
        }
        for (k, v) in ctx.headers {
            header_pairs.push(format!("\"{}\" => \"{}\"", escape_php(k), escape_php(v)));
        }
        if !header_pairs.is_empty() {
            opts.push(format!("'headers' => [{}]", header_pairs.join(", ")));
        }

        if !ctx.cookies.is_empty() {
            let cookie_str = ctx
                .cookies
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("; ");
            opts.push(format!("'headers' => ['Cookie' => \"{}\"]", escape_php(&cookie_str)));
        }

        // `ctx.path` is `/fixtures/{id}{request.path}` and already embeds the
        // fixture's query string when it has one; `ctx.query_params` mirrors it.
        // Guzzle's `query` option overrides any query already on the URI, so
        // emitting both is redundant — only add it when the path carries no
        // query component of its own.
        if !ctx.query_params.is_empty() && !ctx.path.contains('?') {
            let pairs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    format!("\"{}\" => \"{}\"", escape_php(k), escape_php(&val_str))
                })
                .collect();
            opts.push(format!("'query' => [{}]", pairs.join(", ")));
        }

        // The template wraps `path` in double quotes itself, so emit only the escaped
        // contents here — wrapping again produces an invalid `""…""` string literal.
        let path_lit = escape_php(ctx.path);

        let rendered = crate::e2e::template_env::render(
            "php/http_request.jinja",
            minijinja::context! {
                method => method,
                path => path_lit,
                opts => opts,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `$this->assertEquals(status, $response->getStatusCode())`.
    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let rendered = crate::e2e::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => status,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a header assertion using `$response->getHeaderLine(...)` or
    /// `$response->hasHeader(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_key_lit = format!("\"{}\"", escape_php(&header_key));
        let assertion_code = match expected {
            "<<present>>" => {
                format!("$this->assertTrue($response->hasHeader({header_key_lit}));")
            }
            "<<absent>>" => {
                format!("$this->assertFalse($response->hasHeader({header_key_lit}));")
            }
            "<<uuid>>" => {
                format!(
                    "$this->assertMatchesRegularExpression('/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i', $response->getHeaderLine({header_key_lit}));"
                )
            }
            literal => {
                let val_lit = format!("\"{}\"", escape_php(literal));
                format!("$this->assertEquals({val_lit}, $response->getHeaderLine({header_key_lit}));")
            }
        };

        let mut headers = vec![std::collections::HashMap::new()];
        headers[0].insert("assertion_code", assertion_code);

        let rendered = crate::e2e::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => 0u16,
                headers => headers,
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a JSON body equality assertion.
    ///
    /// Plain string bodies are compared against `(string) $response->getBody()` directly;
    /// structured bodies (objects, arrays, booleans, numbers) are decoded via `json_decode`
    /// and compared with `assertEquals`.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        let body_assertion = match expected {
            serde_json::Value::String(s) if !s.is_empty() => {
                let php_val = format!("\"{}\"", escape_php(s));
                format!("$this->assertEquals({php_val}, (string) $response->getBody());")
            }
            _ => {
                let php_val = super::values::json_to_php(expected);
                format!(
                    "$body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);\n        $this->assertEquals({php_val}, $body);"
                )
            }
        };

        let rendered = crate::e2e::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => body_assertion,
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit partial body assertions: one `assertEquals` per field in `expected`.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let mut partial_body: Vec<std::collections::HashMap<&str, String>> = Vec::new();
            for (key, val) in obj {
                let php_key = format!("\"{}\"", escape_php(key));
                let php_val = super::values::json_to_php(val);
                let assertion_code = format!("$this->assertEquals({php_val}, $body[{php_key}]);");
                let mut entry = std::collections::HashMap::new();
                entry.insert("assertion_code", assertion_code);
                partial_body.push(entry);
            }

            let rendered = crate::e2e::template_env::render(
                "php/http_assertions.jinja",
                minijinja::context! {
                    response_var => "",
                    status_code => 0u16,
                    headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                    body_assertion => String::new(),
                    partial_body => partial_body,
                    validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
                },
            );
            out.push_str(&rendered);
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` against the
    /// JSON-encoded body string (PHP binding returns ProblemDetails with `errors` array).
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let mut validation_errors: Vec<std::collections::HashMap<&str, String>> = Vec::new();
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_php(&err.msg));
            let assertion_code =
                format!("$this->assertStringContainsString({msg_lit}, json_encode($body, JSON_UNESCAPED_SLASHES));");
            let mut entry = std::collections::HashMap::new();
            entry.insert("assertion_code", assertion_code);
            validation_errors.push(entry);
        }

        let rendered = crate::e2e::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => validation_errors,
            },
        );
        out.push_str(&rendered);
    }
}

/// Render a PHPUnit test method for an HTTP server test fixture via the shared driver.
///
/// Handles the one PHP-specific pre-condition: HTTP 101 (WebSocket upgrade) causes
/// cURL to fail; it is emitted as a `markTestSkipped` stub directly.
pub(super) fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) causes cURL to treat the connection as an
    // upgrade and fail with "empty reply from server". Skip these tests in the PHP e2e suite
    // since Guzzle cannot assert on WebSocket upgrade responses via regular HTTP.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_filename(&fixture.id);
        let description = &fixture.description;
        out.push_str(&crate::e2e::template_env::render(
            "php/http_test_skip_101.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
            },
        ));
        return;
    }

    client::http_call::render_http_test(out, &PhpTestClientRenderer, fixture);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::client::{CallCtx, TestClientRenderer};
    use std::collections::BTreeMap;

    fn ctx_with<'a>(
        path: &'a str,
        query: &'a BTreeMap<String, serde_json::Value>,
        headers: &'a BTreeMap<String, String>,
        cookies: &'a BTreeMap<String, String>,
    ) -> CallCtx<'a> {
        CallCtx {
            method: "GET",
            path,
            headers,
            query_params: query,
            cookies,
            body: None,
            content_type: None,
            response_var: "response",
        }
    }

    #[test]
    fn render_call_omits_query_option_when_path_already_has_one() {
        let headers = BTreeMap::new();
        let cookies = BTreeMap::new();
        let mut query = BTreeMap::new();
        query.insert("term".to_string(), serde_json::Value::String("hi there".to_string()));
        let ctx = ctx_with("/fixtures/x/search?term=hi%20there", &query, &headers, &cookies);

        let mut out = String::new();
        PhpTestClientRenderer.render_call(&mut out, &ctx);

        assert!(out.contains("/fixtures/x/search?term=hi%20there"), "got: {out}");
        assert!(!out.contains("'query' =>"), "redundant query option emitted: {out}");
    }

    #[test]
    fn render_call_emits_query_option_when_path_has_none() {
        let headers = BTreeMap::new();
        let cookies = BTreeMap::new();
        let mut query = BTreeMap::new();
        query.insert("term".to_string(), serde_json::Value::String("foo".to_string()));
        let ctx = ctx_with("/fixtures/x/search", &query, &headers, &cookies);

        let mut out = String::new();
        PhpTestClientRenderer.render_call(&mut out, &ctx);

        assert!(out.contains("'query' =>"), "expected query option, got: {out}");
        assert!(out.contains("\"term\" => \"foo\""), "got: {out}");
    }
}
