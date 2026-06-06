use crate::e2e::codegen::client;
use crate::e2e::escape::escape_java;
use crate::e2e::fixture::{Fixture, HttpFixture};
use heck::ToUpperCamelCase;

/// Thin renderer that emits JUnit 5 test methods targeting a mock server via
/// `java.net.http.HttpClient`. Satisfies [`client::TestClientRenderer`] so the
/// shared [`client::http_call::render_http_test`] driver drives the call sequence.
pub(super) struct JavaTestClientRenderer;

impl client::TestClientRenderer for JavaTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "java"
    }

    /// Convert a fixture id to the UpperCamelCase suffix appended to `test`.
    ///
    /// The emitted method name is `test{fn_name}`, matching the pre-existing shape.
    fn sanitize_test_name(&self, id: &str) -> String {
        id.to_upper_camel_case()
    }

    /// Emit `@Test void test{fn_name}() throws Exception {`.
    ///
    /// When `skip_reason` is `Some`, the body is a single
    /// `Assumptions.assumeTrue(false, ...)` call and `render_test_close` closes
    /// the brace symmetrically.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_reason = skip_reason.map(escape_java);
        let rendered = crate::e2e::template_env::render(
            "java/http_test_open.jinja",
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
        let rendered = crate::e2e::template_env::render("java/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit a `java.net.http.HttpClient` request to `baseUrl + path`.
    ///
    /// Binds the response to `response` (the `ctx.response_var`). Java's
    /// `HttpClient` disallows a fixed set of restricted headers; those are
    /// silently dropped so the test compiles.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        // Java's HttpClient throws IllegalArgumentException for these headers.
        const JAVA_RESTRICTED_HEADERS: &[&str] = &["connection", "content-length", "expect", "host", "upgrade"];

        let method = ctx.method.to_uppercase();

        // Build the path, appending query params when present.
        let path = if ctx.query_params.is_empty() {
            ctx.path.to_string()
        } else {
            let pairs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    // Percent-encode so values with spaces/reserved characters yield a valid
                    // URI literal (java.net.URI.create rejects raw spaces).
                    format!(
                        "{}={}",
                        super::super::percent_encode_query(k),
                        super::super::percent_encode_query(&val_str)
                    )
                })
                .collect();
            format!("{}?{}", ctx.path, pairs.join("&"))
        };

        let body_publisher = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_java(&json);
            format!("java.net.http.HttpRequest.BodyPublishers.ofString(\"{escaped}\")")
        } else {
            "java.net.http.HttpRequest.BodyPublishers.noBody()".to_string()
        };

        // Content-Type header — only when a body is present.
        let content_type = if ctx.body.is_some() {
            let ct = ctx.content_type.unwrap_or("application/json");
            // Only emit when not already in ctx.headers (avoid duplicate Content-Type).
            if !ctx.headers.keys().any(|k| k.to_lowercase() == "content-type") {
                Some(ct.to_string())
            } else {
                None
            }
        } else {
            None
        };

        // Build header lines — skip Java-restricted ones.
        let mut headers_lines: Vec<String> = Vec::new();
        for (name, value) in ctx.headers {
            if JAVA_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_java(name);
            let escaped_value = escape_java(value);
            headers_lines.push(format!(
                "builder = builder.header(\"{escaped_name}\", \"{escaped_value}\");"
            ));
        }

        // Cookies as a single `Cookie` header.
        let cookies_line = if !ctx.cookies.is_empty() {
            let cookie_str: Vec<String> = ctx.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_java(&cookie_str.join("; "));
            Some(format!("builder = builder.header(\"Cookie\", \"{cookie_header}\");"))
        } else {
            None
        };

        let rendered = crate::e2e::template_env::render(
            "java/http_request.jinja",
            minijinja::context! {
                method => method,
                path => path,
                body_publisher => body_publisher,
                content_type => content_type,
                headers_lines => headers_lines,
                cookies_line => cookies_line,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `assertEquals(status, response.statusCode(), ...)`.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let rendered = crate::e2e::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => status,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a header assertion using `response.headers().firstValue(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_java(name);
        let assertion_code = match expected {
            "<<present>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isPresent(), \"header {escaped_name} should be present\");"
                )
            }
            "<<absent>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isEmpty(), \"header {escaped_name} should be absent\");"
                )
            }
            "<<uuid>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").matches(\"[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\"), \"header {escaped_name} should be a UUID\");"
                )
            }
            literal => {
                let escaped_value = escape_java(literal);
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").contains(\"{escaped_value}\"), \"header {escaped_name} mismatch\");"
                )
            }
        };

        let mut headers = vec![std::collections::HashMap::new()];
        headers[0].insert("assertion_code", assertion_code);

        let rendered = crate::e2e::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => 0u16,
                headers => headers,
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a JSON body equality assertion using Jackson's `MAPPER.readTree`.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let body_assertion = match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_java(&json_str);
                format!(
                    "var bodyJson = MAPPER.readTree({response_var}.body());\n        var expectedJson = MAPPER.readTree(\"{escaped}\");\n        assertEquals(expectedJson, bodyJson, \"body mismatch\");"
                )
            }
            serde_json::Value::String(s) => {
                let escaped = escape_java(s);
                format!("assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");")
            }
            other => {
                let escaped = escape_java(&other.to_string());
                format!("assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");")
            }
        };

        let rendered = crate::e2e::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => body_assertion,
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit partial JSON body assertions: parse once, then assert each expected field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let mut partial_body: Vec<std::collections::HashMap<&str, String>> = Vec::new();
            for (key, val) in obj {
                let escaped_key = escape_java(key);
                let json_str = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = escape_java(&json_str);
                let assertion_code = format!(
                    "assertEquals(MAPPER.readTree(\"{escaped_val}\"), partialJson.get(\"{escaped_key}\"), \"body field '{escaped_key}' mismatch\");"
                );
                let mut entry = std::collections::HashMap::new();
                entry.insert("assertion_code", assertion_code);
                partial_body.push(entry);
            }

            let rendered = crate::e2e::template_env::render(
                "java/http_assertions.jinja",
                minijinja::context! {
                    response_var => response_var,
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

    /// Emit validation-error assertions: parse the body and check each expected message.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
    ) {
        let mut validation_errors: Vec<std::collections::HashMap<&str, String>> = Vec::new();
        for err in errors {
            let escaped_msg = escape_java(&err.msg);
            let assertion_code = format!(
                "assertTrue(veBody.contains(\"{escaped_msg}\"), \"expected validation error message: {escaped_msg}\");"
            );
            let mut entry = std::collections::HashMap::new();
            entry.insert("assertion_code", assertion_code);
            validation_errors.push(entry);
        }

        let rendered = crate::e2e::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
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

/// Render an HTTP server test method using `java.net.http.HttpClient` against
/// `MOCK_SERVER_URL`. Delegates to the shared
/// [`client::http_call::render_http_test`] driver via [`JavaTestClientRenderer`].
///
/// The one Java-specific pre-condition — HTTP 101 (WebSocket upgrade) causing an
/// `EOFException` in `HttpClient` — is handled here before delegating.
pub(super) fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) causes Java's HttpClient to throw EOFException.
    // Emit an assumeTrue(false, ...) stub so the test is skipped rather than failing.
    if http.expected_response.status_code == 101 {
        let method_name = fixture.id.to_upper_camel_case();
        let description = &fixture.description;
        out.push_str(&crate::e2e::template_env::render(
            "java/http_test_skip_101.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
            },
        ));
        return;
    }

    client::http_call::render_http_test(out, &JavaTestClientRenderer, fixture);
}
