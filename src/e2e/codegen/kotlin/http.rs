use crate::e2e::codegen::client;
use crate::e2e::escape::{escape_kotlin, sanitize_ident};
use crate::e2e::fixture::{Fixture, HttpFixture, ValidationErrorExpectation};
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;

// ---------------------------------------------------------------------------
// HTTP server test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// URL-encode special characters in a URI path to prevent URISyntaxException.
/// Only encodes characters that are invalid in URIs: space, pipe, etc.
/// Preserves path structure (/, ?, =, &) and normal alphanumerics.
pub(super) fn url_encode_path(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            // Characters that must be encoded in URI paths per RFC 3986
            '|' => "%7C".to_string(),
            '<' => "%3C".to_string(),
            '>' => "%3E".to_string(),
            '"' => "%22".to_string(),
            '#' => "%23".to_string(),
            '%' => "%25".to_string(),
            // Preserve: alphanumerics, /, -, _, ., ~, ?, =, &, :, @, !
            // These are either unreserved or reserved characters safe in paths/queries
            c if c.is_ascii_alphanumeric() || "/-_.~?=&:@!".contains(c) => c.to_string(),
            // Encode everything else (whitespace, accented chars, control chars, etc.)
            c => format!("%{:02X}", c as u8),
        })
        .collect()
}

/// Renderer that emits JUnit 5 `@Test fun testFoo()` blocks using
/// `java.net.http.HttpClient` against `System.getenv("MOCK_SERVER_URL")`.
pub(super) struct KotlinTestClientRenderer;

impl client::TestClientRenderer for KotlinTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "kotlin"
    }

    /// `java.net.http.HttpClient` performs no content decoding whatsoever — it has no
    /// equivalent of .NET's `AutomaticDecompression` and no transparent gzip path — so an
    /// encoded body reaches Jackson as raw bytes. The JDK ships a gzip decoder
    /// (`GZIPInputStream`) but no brotli one, so brotli could not be added here without a
    /// third-party artifact. Claims neither rather than pretending to gzip it does not apply.
    fn decodable_content_encodings(&self) -> &'static [&'static str] {
        &[]
    }

    fn sanitize_test_name(&self, id: &str) -> String {
        sanitize_ident(id).to_upper_camel_case()
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    fun test{fn_name}() {{");
        let _ = writeln!(out, "        // {description}");
        if let Some(reason) = skip_reason {
            let escaped = escape_kotlin(reason);
            let _ = writeln!(
                out,
                "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"{escaped}\")"
            );
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_path = ctx.path;

        // Java's HttpClient restricts certain headers that cannot be set programmatically.
        const JAVA_RESTRICTED_HEADERS: &[&str] = &["connection", "content-length", "expect", "host", "upgrade"];

        // Resolution order: an externally preset SUT_URL (env or system property,
        // e.g. from `alef test-apps run` pointing at a real bound app) wins.
        // Otherwise fall back to the `mockServerUrl` system property populated by
        // `MockServerListener`'s spawned `mock-server` binary (mirrors the Java
        // backend's `mock_url_list` resolution in args.rs), then the historical
        // hardcoded default as a last resort.
        let _ = writeln!(
            out,
            "        val baseUrl = System.getenv(\"SUT_URL\") ?: System.getProperty(\"SUT_URL\") ?: System.getProperty(\"mockServerUrl\") ?: \"http://127.0.0.1:8007\""
        );
        // fixture_path is already namespaced like /fixtures/delete_remove_resource from http_call
        // URL-encode special characters in the path to avoid URISyntaxException (e.g., pipe → %7C)
        let encoded_path = url_encode_path(fixture_path);
        let _ = writeln!(out, "        val uri = java.net.URI.create(\"$baseUrl{encoded_path}\")");

        let body_publisher = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            // A full literal expression (see `values::kotlin_string_literal`): quoted, or
            // `+`-chunked when the fixture body is long enough to threaten the JVM's
            // 65535-byte constant cap.
            let literal = super::values::kotlin_string_literal(&json);
            format!("java.net.http.HttpRequest.BodyPublishers.ofString({literal})")
        } else {
            "java.net.http.HttpRequest.BodyPublishers.noBody()".to_string()
        };

        let _ = writeln!(out, "        val builder = java.net.http.HttpRequest.newBuilder(uri)");
        let _ = writeln!(out, "            .method(\"{method}\", {body_publisher})");

        // Content-Type header when there is a body.
        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let _ = writeln!(out, "            .header(\"Content-Type\", \"{content_type}\")");
        }

        // Explicit request headers (sorted for deterministic output).
        let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
        header_pairs.sort_by_key(|(k, _)| k.as_str());
        for (name, value) in &header_pairs {
            if JAVA_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_kotlin(name);
            let escaped_value = escape_kotlin(value);
            let _ = writeln!(out, "            .header(\"{escaped_name}\", \"{escaped_value}\")");
        }

        // Cookies as a single Cookie header.
        if !ctx.cookies.is_empty() {
            let mut cookie_pairs: Vec<(&String, &String)> = ctx.cookies.iter().collect();
            cookie_pairs.sort_by_key(|(k, _)| k.as_str());
            let cookie_str: Vec<String> = cookie_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_kotlin(&cookie_str.join("; "));
            let _ = writeln!(out, "            .header(\"Cookie\", \"{cookie_header}\")");
        }

        let _ = writeln!(
            out,
            "        val {} = java.net.http.HttpClient.newHttpClient()",
            ctx.response_var
        );
        let _ = writeln!(
            out,
            "            .send(builder.build(), java.net.http.HttpResponse.BodyHandlers.ofString())"
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "        assertEquals({status}, {response_var}.statusCode(), \"status code mismatch\")"
        );
    }

    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_kotlin(name);
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isPresent, \"header {escaped_name} should be present\")"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "        assertFalse({response_var}.headers().firstValue(\"{escaped_name}\").isPresent, \"header {escaped_name} should be absent\")"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").matches(Regex(\"[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\")), \"header {escaped_name} should be a UUID\")"
                );
            }
            exact => {
                let escaped_value = escape_kotlin(exact);
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").contains(\"{escaped_value}\"), \"header {escaped_name} mismatch\")"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_kotlin(&json_str);
                let _ = writeln!(out, "        val bodyJson = MAPPER.readTree({response_var}.body())");
                let _ = writeln!(out, "        val expectedJson = MAPPER.readTree(\"{escaped}\")");
                let _ = writeln!(out, "        assertEquals(expectedJson, bodyJson, \"body mismatch\")");
            }
            serde_json::Value::String(s) => {
                let escaped = escape_kotlin(s);
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\")"
                );
            }
            other => {
                let escaped = escape_kotlin(&other.to_string());
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\")"
                );
            }
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "        val _partialTree = MAPPER.readTree({response_var}.body())");
            for (key, val) in obj {
                let escaped_key = escape_kotlin(key);
                match val {
                    serde_json::Value::String(s) => {
                        let escaped_val = escape_kotlin(s);
                        let _ = writeln!(
                            out,
                            "        assertEquals(\"{escaped_val}\", _partialTree.path(\"{escaped_key}\").asText(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    serde_json::Value::Bool(b) => {
                        let _ = writeln!(
                            out,
                            "        assertEquals({b}, _partialTree.path(\"{escaped_key}\").asBoolean(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    serde_json::Value::Number(n) => {
                        let _ = writeln!(
                            out,
                            "        assertEquals({n}, _partialTree.path(\"{escaped_key}\").numberValue(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    other => {
                        let json_str = serde_json::to_string(other).unwrap_or_default();
                        let escaped_val = escape_kotlin(&json_str);
                        let _ = writeln!(
                            out,
                            "        assertEquals(MAPPER.readTree(\"{escaped_val}\"), _partialTree.path(\"{escaped_key}\"), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                }
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "        val _veTree = MAPPER.readTree({response_var}.body())");
        let _ = writeln!(out, "        val _veErrors = _veTree.path(\"errors\")");
        for ve in errors {
            let escaped_msg = escape_kotlin(&ve.msg);
            let _ = writeln!(
                out,
                "        assertTrue((0 until _veErrors.size()).any {{ _veErrors.get(it).path(\"msg\").asText().contains(\"{escaped_msg}\") }}, \"expected validation error containing: {escaped_msg}\")"
            );
        }
    }
}

/// Render a JUnit 5 `@Test` method for an HTTP server fixture via the shared driver.
///
/// HTTP 101 (WebSocket upgrade) is emitted as a skip stub because Java's
/// `HttpClient` cannot handle protocol-switch responses (throws `EOFException`).
pub(super) fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) — java.net.http.HttpClient cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_ident(&fixture.id).to_upper_camel_case();
        let description = &fixture.description;
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    fun test{method_name}() {{");
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"Skipped: Java HttpClient cannot handle 101 Switching Protocols responses\")"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    client::http_call::render_http_test(out, &KotlinTestClientRenderer, fixture);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::client::{CallCtx, TestClientRenderer};
    use std::collections::BTreeMap;

    /// Regression: without a preset `SUT_URL`, the base URL must fall back to
    /// the `mockServerUrl` system property populated by the now-wired
    /// `MockServerListener` (spawned `mock-server` binary) before the
    /// hardcoded default — otherwise every request fails with
    /// `ConnectException` against a server that was never started.
    #[test]
    fn render_call_base_url_falls_back_to_mock_server_url_property() {
        let headers = BTreeMap::new();
        let cookies = BTreeMap::new();
        let query = BTreeMap::new();
        let ctx = CallCtx {
            method: "GET",
            path: "/fixtures/x",
            headers: &headers,
            query_params: &query,
            cookies: &cookies,
            body: None,
            content_type: None,
            response_var: "response",
        };

        let mut out = String::new();
        KotlinTestClientRenderer.render_call(&mut out, &ctx);

        assert!(
            out.contains(r#"System.getProperty("mockServerUrl")"#),
            "expected mockServerUrl system property fallback, got: {out}"
        );
        assert!(
            out.contains(r#"System.getenv("SUT_URL")"#),
            "expected SUT_URL env var to still take priority, got: {out}"
        );
    }
}
