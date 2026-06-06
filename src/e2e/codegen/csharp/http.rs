//! C# HTTP e2e test rendering.

use crate::e2e::escape::escape_csharp;
use crate::e2e::fixture::{Fixture, HttpFixture, ValidationErrorExpectation};
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::client;

// ---------------------------------------------------------------------------
// HTTP test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Renderer that emits xUnit `[Fact] public async Task Test_*()` methods using
/// `System.Net.Http.HttpClient` against the mock server at `MOCK_SERVER_URL`.
/// Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct CSharpTestClientRenderer;

/// C# HttpMethod static properties are PascalCase (Get, Post, Put, Delete, …).
fn to_csharp_http_method(method: &str) -> String {
    let lower = method.to_ascii_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Headers that belong to `request.Content.Headers` rather than `request.Headers`.
///
/// Adding these to `request.Headers` causes .NET to throw "Misused header name".
const CSHARP_RESTRICTED_REQUEST_HEADERS: &[&str] = &[
    "content-length",
    "host",
    "connection",
    "expect",
    "transfer-encoding",
    "upgrade",
    // Content-Type is owned by request.Content.Headers and is set when
    // StringContent is constructed; adding it to request.Headers throws.
    "content-type",
    // Other entity headers also belong to request.Content.Headers.
    "content-encoding",
    "content-language",
    "content-location",
    "content-md5",
    "content-range",
    "content-disposition",
];

/// Whether `name` (any case) belongs to `response.Content.Headers` rather than
/// `response.Headers`. Picking the wrong collection causes .NET to throw
/// "Misused header name".
fn is_csharp_content_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-type"
            | "content-length"
            | "content-encoding"
            | "content-language"
            | "content-location"
            | "content-md5"
            | "content-range"
            | "content-disposition"
            | "expires"
            | "last-modified"
            | "allow"
    )
}

impl client::TestClientRenderer for CSharpTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "csharp"
    }

    /// Convert a fixture id to the PascalCase identifier used in `Test_{name}`.
    fn sanitize_test_name(&self, id: &str) -> String {
        id.to_upper_camel_case()
    }

    /// Emit `[Fact]` (or `[Fact(Skip = "…")]` for skipped tests), the method
    /// signature, the opening brace, and the description comment.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_reason = skip_reason.map(escape_csharp);
        let rendered = crate::e2e::template_env::render(
            "csharp/http_test_open.jinja",
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
        let rendered = crate::e2e::template_env::render("csharp/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit the `HttpRequestMessage` construction, headers, cookies, body, and
    /// `var response = await client.SendAsync(request)`.
    ///
    /// The fixture path follows the mock-server convention `/fixtures/<id>`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = to_csharp_http_method(ctx.method);

        // Extract path parameter names from placeholders like {id}, {date}, etc.
        // These will be declared as placeholder variables (empty strings for now)
        let path_param_names = extract_path_param_names(ctx.path);

        out.push_str("        var baseUrl = Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\";\n");

        // Emit declarations for any path parameters found in the URL pattern
        // These are placeholders like {id}, {date} that will be interpolated into the path
        for param_name in &path_param_names {
            out.push_str(&format!("        var {param_name} = \"\";\n"));
        }

        // Disable auto-follow so redirect-status fixtures (3xx) can assert the
        // server's status code rather than the followed-target's status.
        out.push_str(
            "        using var handler = new System.Net.Http.HttpClientHandler { AllowAutoRedirect = false };\n",
        );
        out.push_str("        using var client = new System.Net.Http.HttpClient(handler);\n");
        // Don't escape the path - it contains {param} placeholders that need to be preserved
        // for C# string interpolation
        out.push_str(&format!("        var request = new System.Net.Http.HttpRequestMessage(System.Net.Http.HttpMethod.{method}, $\"{{baseUrl}}{}\");\n", ctx.path));

        // Set body + Content-Type when a request body is present.
        if let Some(body) = ctx.body {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_csharp(&json_str);

            // For multipart/form-data with boundary, use ByteArrayContent with explicit header
            // because StringContent constructor rejects boundary in MediaType.
            if content_type.contains("multipart/form-data") && content_type.contains("boundary=") {
                // Extract the base content type and boundary parameter
                let boundary_pos = content_type.find("boundary=").unwrap_or(0);
                let boundary_value = &content_type[boundary_pos + 9..];

                out.push_str("        var multipartBytes = System.Text.Encoding.UTF8.GetBytes(\"");
                out.push_str(&escaped);
                out.push_str("\");\n");
                out.push_str("        var multipartContent = new System.Net.Http.ByteArrayContent(multipartBytes);\n");
                out.push_str("        var mediaType = new System.Net.Http.Headers.MediaTypeHeaderValue(\"multipart/form-data\");\n");
                out.push_str(&format!("        mediaType.Parameters.Add(new System.Net.Http.Headers.NameValueHeaderValue(\"boundary\", \"{boundary_value}\"));\n"));
                out.push_str("        multipartContent.Headers.ContentType = mediaType;\n");
                out.push_str("        request.Content = multipartContent;\n");
            } else {
                out.push_str(&format!("        request.Content = new System.Net.Http.StringContent(\"{escaped}\", System.Text.Encoding.UTF8, \"{content_type}\");\n"));
            }
        }

        // Add request headers (skip restricted headers that belong to Content.Headers).
        for (name, value) in ctx.headers {
            if CSHARP_RESTRICTED_REQUEST_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_csharp(name);
            let escaped_value = escape_csharp(value);
            out.push_str(&format!(
                "        request.Headers.Add(\"{escaped_name}\", \"{escaped_value}\");\n"
            ));
        }

        // Combine cookies into a single `Cookie` header.
        if !ctx.cookies.is_empty() {
            let mut pairs: Vec<String> = ctx.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            pairs.sort();
            let cookie_header = escape_csharp(&pairs.join("; "));
            out.push_str(&format!(
                "        request.Headers.Add(\"Cookie\", \"{cookie_header}\");\n"
            ));
        }

        out.push_str("        var response = await client.SendAsync(request);\n");
    }

    /// Emit `Assert.Equal(status, (int)response.StatusCode)`.
    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        out.push_str(&format!("        Assert.Equal({status}, (int)response.StatusCode);\n"));
    }

    /// Emit a response-header assertion.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    /// Picks `response.Content.Headers` vs `response.Headers` based on the header name.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let target = if is_csharp_content_header(name) {
            "response.Content.Headers"
        } else {
            "response.Headers"
        };
        let escaped_name = escape_csharp(name);
        match expected {
            "<<present>>" => {
                out.push_str(&format!("        Assert.True({target}.Contains(\"{escaped_name}\"), \"expected header {escaped_name} to be present\");\n"));
            }
            "<<absent>>" => {
                out.push_str(&format!("        Assert.False({target}.Contains(\"{escaped_name}\"), \"expected header {escaped_name} to be absent\");\n"));
            }
            "<<uuid>>" => {
                // UUID regex: 8-4-4-4-12 hex groups.
                out.push_str(&format!("        Assert.True({target}.TryGetValues(\"{escaped_name}\", out var _uuidHdr) && System.Text.RegularExpressions.Regex.IsMatch(string.Join(\", \", _uuidHdr), @\"^[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}$\"), \"header {escaped_name} is not a UUID\");\n"));
            }
            literal => {
                // Use a deterministic local-variable name derived from the header name so
                // multiple header assertions in the same method body do not redeclare.
                let var_name = format!("hdr{}", sanitize_ident(name));
                let escaped_value = escape_csharp(literal);
                out.push_str(&format!("        Assert.True({target}.TryGetValues(\"{escaped_name}\", out var {var_name}) && {var_name}.Any(v => v.Contains(\"{escaped_value}\")), \"header {escaped_name} mismatch\");\n"));
            }
        }
    }

    /// Emit a JSON body equality assertion via `JsonDocument`.
    ///
    /// Plain-string bodies are compared with `Assert.Equal` after trimming.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_csharp(&json_str);
                out.push_str("        var bodyText = await response.Content.ReadAsStringAsync();\n");
                out.push_str("        var body = JsonDocument.Parse(bodyText).RootElement;\n");
                out.push_str(&format!(
                    "        var expectedBody = JsonDocument.Parse(\"{escaped}\").RootElement;\n"
                ));
                out.push_str("        Assert.Equal(expectedBody.GetRawText(), body.GetRawText());\n");
            }
            serde_json::Value::String(s) => {
                let escaped = escape_csharp(s);
                out.push_str("        var bodyText = await response.Content.ReadAsStringAsync();\n");
                out.push_str(&format!("        Assert.Equal(\"{escaped}\", bodyText.Trim());\n"));
            }
            other => {
                let escaped = escape_csharp(&other.to_string());
                out.push_str("        var bodyText = await response.Content.ReadAsStringAsync();\n");
                out.push_str(&format!("        Assert.Equal(\"{escaped}\", bodyText.Trim());\n"));
            }
        }
    }

    /// Emit per-field equality assertions for a partial body match.
    ///
    /// Uses a separate `partialBodyText` local so it does not collide with
    /// `bodyText` if `render_assert_json_body` was also called.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            out.push_str("        var partialBodyText = await response.Content.ReadAsStringAsync();\n");
            out.push_str("        var partialBody = JsonDocument.Parse(partialBodyText).RootElement;\n");
            for (key, val) in obj {
                let escaped_key = escape_csharp(key);
                let json_str = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = escape_csharp(&json_str);
                let var_name = format!("expected{}", key.to_upper_camel_case());
                out.push_str(&format!(
                    "        var {var_name} = JsonDocument.Parse(\"{escaped_val}\").RootElement;\n"
                ));
                out.push_str(&format!("        Assert.True(partialBody.TryGetProperty(\"{escaped_key}\", out var _partialProp{var_name}) && _partialProp{var_name}.GetRawText() == {var_name}.GetRawText(), \"partial body field '{escaped_key}' mismatch\");\n"));
            }
        }
    }

    /// Emit validation-error assertions by checking each expected `msg` string
    /// appears in the JSON-encoded body.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        out.push_str("        var validationBodyText = await response.Content.ReadAsStringAsync();\n");
        for err in errors {
            let escaped_msg = escape_csharp(&err.msg);
            out.push_str(&format!(
                "        Assert.Contains(\"{escaped_msg}\", validationBodyText);\n"
            ));
        }
    }
}

/// Render an HTTP server test method using the shared [`client::http_call::render_http_test`]
/// driver via [`CSharpTestClientRenderer`].
pub(super) fn render_http_test_method(out: &mut String, fixture: &Fixture, _http: &HttpFixture) {
    client::http_call::render_http_test(out, &CSharpTestClientRenderer, fixture);
}

/// Extract path parameter names from a URL pattern like `/fixtures/{id}/items/{item_id}`.
/// Returns parameter names such as `["id", "item_id"]`, stripping any type syntax like `:uuid`.
fn extract_path_param_names(path: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut in_param = false;
    let mut current_param = String::new();

    for ch in path.chars() {
        match ch {
            '{' => {
                in_param = true;
                current_param.clear();
            }
            '}' => {
                if in_param && !current_param.is_empty() {
                    // Strip type syntax: {id:uuid} → just "id"
                    let param_name = current_param.split(':').next().unwrap_or("").to_string();
                    if !param_name.is_empty() {
                        params.push(param_name);
                    }
                }
                in_param = false;
                current_param.clear();
            }
            _ if in_param => {
                current_param.push(ch);
            }
            _ => {}
        }
    }

    params
}
