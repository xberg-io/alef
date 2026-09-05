//! HTTP integration test generation for Rust e2e tests.

use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::client::http_call::plan_request;
use crate::e2e::escape::{escape_rust, rust_raw_string};
use crate::e2e::fixture::{CorsConfig, Fixture, HttpExpectedResponse, StaticFilesConfig};

/// Response headers that the transport or a response-encoding layer computes for itself.
/// The generated handler must neither set nor assert them: `content-length` and the
/// framing headers are recomputed downstream, and `content-encoding` only appears when a
/// compression layer is wired, which this backend does not do. The sibling backends skip
/// `content-encoding` for the same reason. ~keep
const TRANSPORT_OWNED_HEADERS: [&str; 5] = [
    "content-encoding",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
];

/// Value emitted for a `<<present>>` header expectation — the test only asserts presence.
const PRESENT_HEADER_PLACEHOLDER: &str = "present";

/// Value emitted for a `<<uuid>>` header expectation; the test re-checks its shape rather
/// than its bytes, so any well-formed hyphenated hex UUID would do.
const UUID_HEADER_PLACEHOLDER: &str = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6";

/// How to call a method on axum_test::TestServer in generated code.
enum ServerCall<'a> {
    /// Emit `server.get(path)` / `server.post(path)` etc.
    Shorthand(&'a str),
    /// Emit `server.method(axum::http::Method::OPTIONS, path)` etc.
    AxumMethod(&'a str),
}

/// How the generated test compares the response body against the fixture expectation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedBody {
    /// Compare the response text verbatim against a string literal.
    Text(String),
    /// Parse both sides and compare as JSON values, so key order and whitespace do not matter.
    Json(String),
}

impl ExpectedBody {
    /// The exact bytes the generated handler returns for this expectation.
    fn wire_text(&self) -> &str {
        match self {
            ExpectedBody::Text(text) | ExpectedBody::Json(text) => text,
        }
    }
}

/// Resolve the body expectation for a fixture, or `None` when there is nothing to assert.
///
/// Absent, `null` and empty-string bodies carry no expectation — the same rule the other
/// backends apply. A JSON string body is compared as raw text because that is what a server
/// puts on the wire; every other JSON value is compared structurally.
pub fn plan_expected_body(body: Option<&serde_json::Value>) -> Option<ExpectedBody> {
    match body? {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) if text.is_empty() => None,
        serde_json::Value::String(text) => Some(ExpectedBody::Text(text.clone())),
        structured => serde_json::to_string(structured).ok().map(ExpectedBody::Json),
    }
}

/// Render the Rust literal for the request body a fixture sends.
///
/// A JSON string body is a raw payload — form-urlencoded, multipart, XML, or a deliberately
/// malformed JSON document — and must reach the server byte for byte. Serializing it as JSON
/// would wrap it in quotes and change what is under test: a malformed-JSON payload becomes a
/// valid JSON string, and a form body gains two stray characters that shift every field. It is
/// escaped into a regular string literal rather than a raw one so that `\r\n` and other control
/// characters survive, which multipart bodies depend on. ~keep
pub fn render_request_body_literal(body: &serde_json::Value) -> String {
    match body {
        serde_json::Value::String(text) => format!("\"{}\"", escape_rust(text)),
        structured => {
            let json = serde_json::to_string(structured).unwrap_or_else(|_| "{}".to_string());
            rust_raw_string(&json)
        }
    }
}

/// The check applied to a single expected response header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderCheck {
    /// The header must equal this exact value.
    Exact(String),
    /// The header must be set, whatever its value (`<<present>>`).
    Present,
    /// The header must not be set (`<<absent>>`).
    Absent,
    /// The header must hold a hyphenated hex UUID (`<<uuid>>`).
    Uuid,
}

/// A planned header expectation with its name already normalised to lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedHeader {
    pub name: String,
    pub check: HeaderCheck,
}

impl ExpectedHeader {
    /// The value the generated handler sets for this header, or `None` when the handler
    /// must leave it unset.
    fn handler_value(&self) -> Option<&str> {
        match &self.check {
            HeaderCheck::Exact(value) => Some(value),
            HeaderCheck::Present => Some(PRESENT_HEADER_PLACEHOLDER),
            HeaderCheck::Uuid => Some(UUID_HEADER_PLACEHOLDER),
            HeaderCheck::Absent => None,
        }
    }
}

/// Returns true for headers a CORS layer writes itself. When one is applied, the handler
/// must not also set them or the response carries duplicates. ~keep
fn is_cors_owned_header(name: &str) -> bool {
    name.starts_with("access-control-") || name == "vary"
}

/// Plan the header expectations for a fixture, dropping the ones this backend cannot own.
///
/// Names are lowercased so the comparison against the response is case-insensitive, matching
/// the sibling backends.
pub fn plan_expected_headers(headers: &BTreeMap<String, String>, cors_applied: bool) -> Vec<ExpectedHeader> {
    let mut planned = Vec::new();
    for (raw_name, value) in headers {
        let name = raw_name.to_lowercase();
        if TRANSPORT_OWNED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if cors_applied && is_cors_owned_header(&name) {
            continue;
        }
        let check = match value.as_str() {
            "<<present>>" => HeaderCheck::Present,
            "<<absent>>" => HeaderCheck::Absent,
            "<<uuid>>" => HeaderCheck::Uuid,
            exact => HeaderCheck::Exact(exact.to_string()),
        };
        planned.push(ExpectedHeader { name, check });
    }
    planned
}

/// Emit the body assertion for a resolved body expectation.
pub fn render_body_assertion(out: &mut String, expected: &ExpectedBody) {
    match expected {
        ExpectedBody::Text(text) => {
            let literal = rust_raw_string(text);
            let _ = writeln!(out, "    assert_eq!(response.text(), {literal});");
        }
        ExpectedBody::Json(json) => {
            let literal = rust_raw_string(json);
            let _ = writeln!(
                out,
                "    let expected_json: serde_json::Value = serde_json::from_str({literal}).expect(\"fixture body is valid JSON\");"
            );
            let _ = writeln!(
                out,
                "    let actual_json: serde_json::Value = serde_json::from_str(&response.text()).expect(\"response body is valid JSON\");"
            );
            let _ = writeln!(out, "    assert_eq!(actual_json, expected_json);");
        }
    }
}

/// Emit the assertions for a planned set of header expectations.
pub fn render_header_assertions(out: &mut String, planned: &[ExpectedHeader]) {
    for header in planned {
        let name = &header.name;
        match &header.check {
            HeaderCheck::Exact(value) => {
                let literal = rust_raw_string(value);
                let _ = writeln!(
                    out,
                    "    assert_eq!(response.headers().get({name:?}).and_then(|v| v.to_str().ok()), Some({literal}), \"unexpected value for header '{name}'\");"
                );
            }
            HeaderCheck::Present => {
                let _ = writeln!(
                    out,
                    "    assert!(response.headers().contains_key({name:?}), \"expected header '{name}' to be present\");"
                );
            }
            HeaderCheck::Absent => {
                let _ = writeln!(
                    out,
                    "    assert!(!response.headers().contains_key({name:?}), \"expected header '{name}' to be absent\");"
                );
            }
            HeaderCheck::Uuid => {
                let _ = writeln!(
                    out,
                    "    let header_value = response.headers().get({name:?}).and_then(|v| v.to_str().ok()).unwrap_or_default();"
                );
                let _ = writeln!(out, "    assert!(");
                let _ = writeln!(out, "        header_value.len() == 36");
                let _ = writeln!(out, "            && header_value.chars().enumerate().all(|(i, c)| {{");
                let _ = writeln!(
                    out,
                    "                if matches!(i, 8 | 13 | 18 | 23) {{ c == '-' }} else {{ c.is_ascii_hexdigit() }}"
                );
                let _ = writeln!(out, "            }}),");
                let _ = writeln!(
                    out,
                    "        \"expected header '{name}' to be a UUID, got {{header_value:?}}\""
                );
                let _ = writeln!(out, "    );");
            }
        }
    }
}

/// Emit the `.header(...)` builder calls the generated handler needs to satisfy the planned
/// header expectations, plus the default `content-type` when the fixture does not pin one.
fn render_handler_headers(out: &mut String, planned: &[ExpectedHeader]) {
    let pins_content_type = planned.iter().any(|header| header.name == "content-type");
    if !pins_content_type {
        let _ = writeln!(out, "                .header(\"content-type\", \"application/json\")");
    }
    for header in planned {
        if let Some(value) = header.handler_value() {
            let name = &header.name;
            let literal = rust_raw_string(value);
            let _ = writeln!(out, "                .header({name:?}, {literal})");
        }
    }
}

/// Resolve what a fixture's response assertions should cover.
///
/// A CORS layer answers a preflight request itself without ever invoking the handler, so
/// neither the handler's body nor its headers reach the client. Asserting them would test
/// the layer's short-circuit, not the fixture.
fn plan_response_assertions(
    expected: &HttpExpectedResponse,
    request_method: &str,
    cors_applied: bool,
) -> (Option<ExpectedBody>, Vec<ExpectedHeader>) {
    if cors_applied && request_method.eq_ignore_ascii_case("OPTIONS") {
        return (None, Vec::new());
    }
    (
        plan_expected_body(expected.body.as_ref()),
        plan_expected_headers(&expected.headers, cors_applied),
    )
}

/// How to register a route on the configured HTTP framework's App in generated code.
enum RouteRegistration<'a> {
    /// Emit `<dep>::get(path)` / `<dep>::post(path)` etc.
    Shorthand(&'a str),
    /// Emit `<dep>::RouteBuilder::new(<dep>::Method::Options, path)` etc.
    Explicit(&'a str),
}

/// Generate a complete integration test function for an http fixture.
///
/// Builds a real `App` from the configured HTTP framework crate with a handler
/// that returns the expected response, then uses `axum_test::TestServer` to send
/// the request and assert the status code.
pub fn render_http_test_function(out: &mut String, fixture: &Fixture, dep_name: &str) {
    let http = match &fixture.http {
        Some(h) => h,
        None => return,
    };

    let fn_name = crate::e2e::escape::sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let route = &http.handler.route;

    // The configured HTTP framework crate is expected to expose convenience functions
    // for GET/POST/PUT/PATCH/DELETE. All other methods (HEAD, OPTIONS, TRACE, etc.) must
    // use RouteBuilder::new directly.
    let route_reg = match http.handler.method.to_lowercase().as_str() {
        "get" => RouteRegistration::Shorthand("get"),
        "post" => RouteRegistration::Shorthand("post"),
        "put" => RouteRegistration::Shorthand("put"),
        "patch" => RouteRegistration::Shorthand("patch"),
        "delete" => RouteRegistration::Shorthand("delete"),
        "head" => RouteRegistration::Explicit("Head"),
        "options" => RouteRegistration::Explicit("Options"),
        "trace" => RouteRegistration::Explicit("Trace"),
        _ => RouteRegistration::Shorthand("get"),
    };

    // axum_test::TestServer has shorthand methods for GET/POST/PUT/PATCH/DELETE.
    // For HEAD and other methods, use server.method(axum::http::Method::HEAD, path).
    let server_call = match http.request.method.to_uppercase().as_str() {
        "GET" => ServerCall::Shorthand("get"),
        "POST" => ServerCall::Shorthand("post"),
        "PUT" => ServerCall::Shorthand("put"),
        "PATCH" => ServerCall::Shorthand("patch"),
        "DELETE" => ServerCall::Shorthand("delete"),
        "HEAD" => ServerCall::AxumMethod("HEAD"),
        "OPTIONS" => ServerCall::AxumMethod("OPTIONS"),
        "TRACE" => ServerCall::AxumMethod("TRACE"),
        _ => ServerCall::Shorthand("get"),
    };

    let req_path = &http.request.path;
    let status = http.expected_response.status_code;

    let request_plan = plan_request(http);
    let request_body_literal = request_plan.body.as_ref().map(render_request_body_literal);
    let mut request_headers = request_plan.headers.clone();
    if let Some(content_type) = request_plan.content_type.as_ref()
        && !request_headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        request_headers.insert("Content-Type".to_string(), content_type.clone());
    }

    // Extract middleware from handler (if any).
    let middleware = http.handler.middleware.as_ref();
    let cors_cfg: Option<&CorsConfig> = middleware.and_then(|m| m.cors.as_ref());
    let static_files_cfgs: Option<&Vec<StaticFilesConfig>> = middleware.and_then(|m| m.static_files.as_ref());
    let has_static_files = static_files_cfgs.is_some_and(|v| !v.is_empty());

    // The handler always echoes the fixture's body, even where the assertions below are
    // suppressed, so the response the framework routes stays faithful to the fixture.
    let fixture_body = plan_expected_body(http.expected_response.body.as_ref());
    let body_literal = rust_raw_string(fixture_body.as_ref().map(ExpectedBody::wire_text).unwrap_or_default());
    let (expected_body, expected_headers) =
        plan_response_assertions(&http.expected_response, &http.request.method, cors_cfg.is_some());

    let _ = writeln!(out, "#[test]");
    let _ = writeln!(out, "fn test_{fn_name}() {{");
    let _ = writeln!(out, "    common::runtime().block_on(async {{");
    let _ = writeln!(out, "    // {description}");

    // When static-files middleware is configured, serve from a temp dir via ServeDir.
    if has_static_files {
        render_static_files_test(out, fixture, static_files_cfgs.unwrap(), &server_call, req_path, status);
        return;
    }

    // Build handler that returns the expected response.
    let _ = writeln!(out, "    let expected_body = {body_literal}.to_string();");
    let _ = writeln!(out, "    let mut app = {dep_name}::App::new();");

    // Emit route registration.
    match &route_reg {
        RouteRegistration::Shorthand(method) => {
            let _ = writeln!(
                out,
                "    app.route({dep_name}::{method}({route:?}), move |_ctx: {dep_name}::RequestContext| {{"
            );
        }
        RouteRegistration::Explicit(variant) => {
            let _ = writeln!(
                out,
                "    app.route({dep_name}::RouteBuilder::new({dep_name}::Method::{variant}, {route:?}), move |_ctx: {dep_name}::RequestContext| {{"
            );
        }
    }
    let _ = writeln!(out, "        let body = expected_body.clone();");
    let _ = writeln!(out, "        async move {{");
    let _ = writeln!(out, "            Ok(axum::http::Response::builder()");
    let _ = writeln!(out, "                .status({status}u16)");
    render_handler_headers(out, &expected_headers);
    let _ = writeln!(out, "                .body(axum::body::Body::from(body))");
    let _ = writeln!(out, "                .unwrap())");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}).unwrap();");

    // Build axum-test TestServer from the app router, optionally wrapping with CorsLayer.
    let _ = writeln!(out, "    let router = app.into_router().unwrap();");
    if let Some(cors) = cors_cfg {
        render_cors_layer(out, cors);
    }
    let _ = writeln!(out, "    let server = axum_test::TestServer::new(router);");

    // Build and send the request.
    match &server_call {
        ServerCall::Shorthand(method) => {
            let _ = writeln!(out, "    let response = server.{method}({req_path:?})");
        }
        ServerCall::AxumMethod(method) => {
            let _ = writeln!(
                out,
                "    let response = server.method(axum::http::Method::{method}, {req_path:?})"
            );
        }
    }

    // Add request headers (axum_test::TestRequest::add_header accepts &str via TryInto).
    for (name, value) in &request_headers {
        let n = rust_raw_string(name);
        let v = rust_raw_string(value);
        let _ = writeln!(out, "        .add_header({n}, {v})");
    }

    // Add request body if present (axum-test's bytes() API takes a Bytes value).
    if let Some(literal) = &request_body_literal {
        let _ = writeln!(
            out,
            "        .bytes(bytes::Bytes::copy_from_slice({literal}.as_bytes()))"
        );
    }

    let _ = writeln!(out, "        .await;");

    // Assert status code.
    // When a CorsLayer is applied and the fixture expects a 2xx status, tower-http may
    // return 200 instead of 204 for preflight. Accept any 2xx status in that case.
    if cors_cfg.is_some() && (200..300).contains(&status) {
        let _ = writeln!(
            out,
            "    assert!(response.status_code().is_success(), \"expected CORS success status, got {{}}\", response.status_code());"
        );
    } else {
        let _ = writeln!(out, "    assert_eq!(response.status_code().as_u16(), {status}u16);");
    }

    if let Some(expected) = &expected_body {
        render_body_assertion(out, expected);
    }
    render_header_assertions(out, &expected_headers);

    let _ = writeln!(out, "    }});");
    let _ = writeln!(out, "}}");
}

/// Emit lines that wrap the axum router with a `tower_http::cors::CorsLayer`.
///
/// The CORS policy is derived from the fixture's `cors` middleware config.
/// After this function, `router` is reassigned to the layer-wrapped version.
pub fn render_cors_layer(out: &mut String, cors: &CorsConfig) {
    // Decide up-front which axum::http re-exports we will actually reference so we
    // can emit a tight `use` group — emitting all three unconditionally trips
    // `-D unused_imports` for fixtures that, say, allow no custom headers.
    let needs_header_value = !cors.allow_origins.is_empty();
    let needs_method = !cors.allow_methods.is_empty();
    let needs_header_name = !cors.allow_headers.is_empty()
        && cors
            .allow_headers
            .iter()
            .any(|h| !matches!(h.to_lowercase().as_str(), "content-type" | "authorization" | "accept"));

    let _ = writeln!(
        out,
        "    // Apply CorsLayer from tower-http based on fixture CORS config."
    );
    let _ = writeln!(out, "    use tower_http::cors::CorsLayer;");
    let mut imports: Vec<&'static str> = Vec::new();
    if needs_header_name {
        imports.push("HeaderName");
    }
    if needs_header_value {
        imports.push("HeaderValue");
    }
    if needs_method {
        imports.push("Method");
    }
    match imports.len() {
        0 => {}
        1 => {
            let _ = writeln!(out, "    use axum::http::{};", imports[0]);
        }
        _ => {
            let _ = writeln!(out, "    use axum::http::{{{}}};", imports.join(", "));
        }
    }
    let _ = writeln!(out, "    let cors_layer = CorsLayer::new()");

    // allow_origins
    if cors.allow_origins.is_empty() {
        let _ = writeln!(out, "        .allow_origin(tower_http::cors::Any)");
    } else {
        let _ = writeln!(out, "        .allow_origin([");
        for origin in &cors.allow_origins {
            let _ = writeln!(out, "            \"{origin}\".parse::<HeaderValue>().unwrap(),");
        }
        let _ = writeln!(out, "        ])");
    }

    // allow_methods
    if cors.allow_methods.is_empty() {
        let _ = writeln!(out, "        .allow_methods(tower_http::cors::Any)");
    } else {
        let methods: Vec<String> = cors
            .allow_methods
            .iter()
            .map(|m| format!("Method::{}", m.to_uppercase()))
            .collect();
        let _ = writeln!(out, "        .allow_methods([{}])", methods.join(", "));
    }

    // allow_headers
    if cors.allow_headers.is_empty() {
        let _ = writeln!(out, "        .allow_headers(tower_http::cors::Any)");
    } else {
        let headers: Vec<String> = cors
            .allow_headers
            .iter()
            .map(|h| {
                let lower = h.to_lowercase();
                match lower.as_str() {
                    "content-type" => "axum::http::header::CONTENT_TYPE".to_string(),
                    "authorization" => "axum::http::header::AUTHORIZATION".to_string(),
                    "accept" => "axum::http::header::ACCEPT".to_string(),
                    _ => format!("HeaderName::from_static(\"{lower}\")"),
                }
            })
            .collect();
        let _ = writeln!(out, "        .allow_headers([{}])", headers.join(", "));
    }

    // max_age
    if let Some(secs) = cors.max_age {
        let _ = writeln!(out, "        .max_age(std::time::Duration::from_secs({secs}));");
    } else {
        let _ = writeln!(out, "        ;");
    }

    let _ = writeln!(out, "    let router = router.layer(cors_layer);");
}

/// Emit lines for a static-files integration test.
///
/// Writes fixture files to a temporary directory and serves them via
/// `tower_http::services::ServeDir`, bypassing the framework App entirely.
fn render_static_files_test(
    out: &mut String,
    fixture: &Fixture,
    cfgs: &[StaticFilesConfig],
    server_call: &ServerCall<'_>,
    req_path: &str,
    status: u16,
) {
    let http = fixture.http.as_ref().unwrap();

    let _ = writeln!(out, "    use tower_http::services::ServeDir;");
    let _ = writeln!(out, "    use axum::Router;");
    let _ = writeln!(out, "    let tmp_dir = tempfile::tempdir().expect(\"tmp dir\");");

    // Build the router by nesting a ServeDir for each config entry.
    let _ = writeln!(out, "    let mut router = Router::new();");
    for cfg in cfgs {
        for file in &cfg.files {
            let file_path = file.path.replace('\\', "/");
            let content = rust_raw_string(&file.content);
            if file_path.contains('/') {
                let parent: String = file_path.rsplitn(2, '/').last().unwrap_or("").to_string();
                let _ = writeln!(
                    out,
                    "    std::fs::create_dir_all(tmp_dir.path().join(\"{parent}\")).unwrap();"
                );
            }
            let _ = writeln!(
                out,
                "    std::fs::write(tmp_dir.path().join(\"{file_path}\"), {content}).unwrap();"
            );
        }
        let prefix = &cfg.route_prefix;
        let serve_dir_expr = if cfg.index_file {
            "ServeDir::new(tmp_dir.path()).append_index_html_on_directories(true)".to_string()
        } else {
            "ServeDir::new(tmp_dir.path())".to_string()
        };
        let _ = writeln!(out, "    router = router.nest_service({prefix:?}, {serve_dir_expr});");
    }

    let _ = writeln!(out, "    let server = axum_test::TestServer::new(router);");

    // Build and send the request.
    match server_call {
        ServerCall::Shorthand(method) => {
            let _ = writeln!(out, "    let response = server.{method}({req_path:?})");
        }
        ServerCall::AxumMethod(method) => {
            let _ = writeln!(
                out,
                "    let response = server.method(axum::http::Method::{method}, {req_path:?})"
            );
        }
    }

    // Add request headers.
    for (name, value) in &http.request.headers {
        let n = rust_raw_string(name);
        let v = rust_raw_string(value);
        let _ = writeln!(out, "        .add_header({n}, {v})");
    }

    let _ = writeln!(out, "        .await;");
    let _ = writeln!(out, "    assert_eq!(response.status_code().as_u16(), {status}u16);");

    // Only a successful static-file response carries the served content; error responses are
    // synthesised by the file service and have no body to compare. Response headers are left
    // unasserted because the file service — not the fixture — decides them. ~keep
    if (200..300).contains(&status)
        && let Some(expected) = plan_expected_body(http.expected_response.body.as_ref())
    {
        render_body_assertion(out, &expected);
    }

    let _ = writeln!(out, "    }});");
    let _ = writeln!(out, "}}");
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::e2e::fixture::{HttpFixture, HttpHandler, HttpMiddleware, HttpRequest};

    fn http_fixture(expected: HttpExpectedResponse, middleware: Option<HttpMiddleware>, method: &str) -> Fixture {
        Fixture {
            id: "sample".to_string(),
            description: "A sample HTTP test".to_string(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/sample".to_string(),
                    method: method.to_string(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware,
                },
                request: HttpRequest {
                    method: method.to_string(),
                    path: "/sample".to_string(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: expected,
            }),
            ..Fixture::default()
        }
    }

    fn expected_response(body: Option<serde_json::Value>, headers: &[(&str, &str)]) -> HttpExpectedResponse {
        HttpExpectedResponse {
            status_code: 200,
            body,
            body_partial: None,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            validation_errors: None,
        }
    }

    #[test]
    fn plan_expected_body_returns_none_for_absent_null_and_empty_bodies() {
        assert_eq!(plan_expected_body(None), None);
        assert_eq!(plan_expected_body(Some(&serde_json::Value::Null)), None);
        assert_eq!(plan_expected_body(Some(&serde_json::json!(""))), None);
    }

    #[test]
    fn plan_expected_body_compares_string_bodies_as_raw_text() {
        let body = serde_json::json!("Hello from static storage");
        assert_eq!(
            plan_expected_body(Some(&body)),
            Some(ExpectedBody::Text("Hello from static storage".to_string()))
        );
    }

    #[test]
    fn plan_expected_body_compares_structured_bodies_as_json() {
        let body = serde_json::json!({"status": "ok"});
        assert_eq!(
            plan_expected_body(Some(&body)),
            Some(ExpectedBody::Json(r#"{"status":"ok"}"#.to_string()))
        );
    }

    #[test]
    fn render_body_assertion_for_text_compares_response_text() {
        let mut out = String::new();
        render_body_assertion(&mut out, &ExpectedBody::Text("plain".to_string()));
        assert_eq!(out, "    assert_eq!(response.text(), r#\"plain\"#);\n");
    }

    #[test]
    fn render_body_assertion_for_json_compares_parsed_values() {
        let mut out = String::new();
        render_body_assertion(&mut out, &ExpectedBody::Json(r#"{"status":"ok"}"#.to_string()));
        assert!(out.contains("let expected_json: serde_json::Value = serde_json::from_str"));
        assert!(out.contains("serde_json::from_str(&response.text())"));
        assert!(out.contains("assert_eq!(actual_json, expected_json);"));
    }

    /// A form body must reach the server byte for byte. Serializing it as JSON adds a
    /// leading and trailing quote, which becomes part of the first and last field and
    /// silently changes what the fixture tests.
    #[test]
    fn render_request_body_literal_sends_a_string_body_verbatim() {
        let body = serde_json::json!("items[0]=first&items[2]=third");
        assert_eq!(render_request_body_literal(&body), r#""items[0]=first&items[2]=third""#);
    }

    /// Multipart bodies are delimited by CRLF, so the escapes must survive into the
    /// generated literal — a raw string literal would not preserve them.
    #[test]
    fn render_request_body_literal_preserves_control_characters_and_quotes() {
        let body = serde_json::json!("--b\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nx\r\n--b--\r\n");
        let literal = render_request_body_literal(&body);
        assert!(literal.starts_with('"') && literal.ends_with('"'));
        assert!(literal.contains(r"\r\n"), "CRLF must stay escaped: {literal}");
        assert!(
            literal.contains(r#"name=\"file\""#),
            "quotes must stay escaped: {literal}"
        );
        assert!(!literal.contains('\r'), "no bare CR may reach the source: {literal}");
    }

    /// A deliberately malformed JSON payload must stay malformed; wrapping it in quotes
    /// would turn it into a valid JSON string and defeat the fixture.
    #[test]
    fn render_request_body_literal_keeps_a_malformed_json_payload_malformed() {
        let body = serde_json::json!(r#"{"name": "Item", "price": }"#);
        let literal = render_request_body_literal(&body);
        assert_eq!(literal, r#""{\"name\": \"Item\", \"price\": }""#);
    }

    #[test]
    fn render_request_body_literal_serializes_structured_bodies() {
        let body = serde_json::json!({"name": "Item"});
        assert_eq!(render_request_body_literal(&body), r##"r#"{"name":"Item"}"#"##);
    }

    #[test]
    fn render_http_test_function_sends_a_string_request_body_unquoted() {
        let mut fixture = http_fixture(expected_response(None, &[]), None, "POST");
        let http = fixture.http.as_mut().unwrap();
        http.request.body = Some(serde_json::json!("username=johndoe&password=secret"));
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(out.contains(r#"copy_from_slice("username=johndoe&password=secret".as_bytes())"#));
    }

    #[test]
    fn render_http_test_function_omits_the_body_call_when_there_is_no_request_body() {
        let fixture = http_fixture(expected_response(None, &[]), None, "GET");
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(!out.contains("copy_from_slice"));
    }

    #[test]
    fn render_http_test_function_synthesizes_schema_only_multipart_request() {
        let mut fixture = http_fixture(expected_response(None, &[]), None, "POST");
        let http = fixture.http.as_mut().unwrap();
        http.request.content_type = Some("multipart/form-data".into());
        http.handler.body_schema = Some(serde_json::json!({
            "type": "object",
            "properties": { "file": { "type": "string", "format": "binary" } },
        }));

        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(out.contains("boundary=alef-boundary"));
        assert!(out.contains(r"\r\nContent-Disposition: form-data"));
        assert!(out.contains("copy_from_slice"));
    }

    #[test]
    fn render_http_test_function_omits_explicit_empty_multipart_request() {
        let mut fixture = http_fixture(expected_response(None, &[]), None, "POST");
        let http = fixture.http.as_mut().unwrap();
        http.request.content_type = Some("multipart/form-data".into());
        http.request.form_data = Some(BTreeMap::new());
        http.handler.body_schema = Some(serde_json::json!({
            "type": "object",
            "properties": { "file": { "type": "string", "format": "binary" } },
        }));

        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(!out.contains("copy_from_slice"));
        assert!(!out.contains("multipart/form-data"));
    }

    #[test]
    fn plan_expected_headers_lowercases_names_for_case_insensitive_comparison() {
        let headers = BTreeMap::from([("X-Total-Count".to_string(), "42".to_string())]);
        let planned = plan_expected_headers(&headers, false);
        assert_eq!(
            planned,
            vec![ExpectedHeader {
                name: "x-total-count".to_string(),
                check: HeaderCheck::Exact("42".to_string()),
            }]
        );
    }

    #[test]
    fn plan_expected_headers_maps_the_sentinel_tokens() {
        let headers = BTreeMap::from([
            ("a-present".to_string(), "<<present>>".to_string()),
            ("b-absent".to_string(), "<<absent>>".to_string()),
            ("c-uuid".to_string(), "<<uuid>>".to_string()),
        ]);
        let checks: Vec<HeaderCheck> = plan_expected_headers(&headers, false)
            .into_iter()
            .map(|header| header.check)
            .collect();
        assert_eq!(
            checks,
            vec![HeaderCheck::Present, HeaderCheck::Absent, HeaderCheck::Uuid]
        );
    }

    #[test]
    fn plan_expected_headers_drops_transport_owned_headers() {
        let headers = BTreeMap::from([
            ("Content-Encoding".to_string(), "gzip".to_string()),
            ("Content-Length".to_string(), "85".to_string()),
            ("Connection".to_string(), "close".to_string()),
            ("X-Kept".to_string(), "yes".to_string()),
        ]);
        let planned = plan_expected_headers(&headers, false);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].name, "x-kept");
    }

    #[test]
    fn plan_expected_headers_keeps_cors_headers_when_no_cors_layer_is_applied() {
        let headers = BTreeMap::from([("Access-Control-Allow-Origin".to_string(), "https://x.dev".to_string())]);
        assert_eq!(plan_expected_headers(&headers, false).len(), 1);
    }

    #[test]
    fn plan_expected_headers_drops_cors_owned_headers_when_a_cors_layer_is_applied() {
        let headers = BTreeMap::from([
            ("Access-Control-Allow-Origin".to_string(), "https://x.dev".to_string()),
            ("Vary".to_string(), "Origin".to_string()),
            ("X-Total-Count".to_string(), "42".to_string()),
        ]);
        let planned = plan_expected_headers(&headers, true);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].name, "x-total-count");
    }

    #[test]
    fn render_header_assertions_emits_an_equality_check_for_an_exact_value() {
        let mut out = String::new();
        render_header_assertions(
            &mut out,
            &[ExpectedHeader {
                name: "x-total-count".to_string(),
                check: HeaderCheck::Exact("42".to_string()),
            }],
        );
        assert!(out.contains(r#"response.headers().get("x-total-count").and_then(|v| v.to_str().ok())"#));
        assert!(out.contains(r##"Some(r#"42"#)"##));
        assert!(out.contains("unexpected value for header 'x-total-count'"));
    }

    #[test]
    fn render_header_assertions_emits_presence_and_absence_checks() {
        let mut out = String::new();
        render_header_assertions(
            &mut out,
            &[
                ExpectedHeader {
                    name: "set-cookie".to_string(),
                    check: HeaderCheck::Present,
                },
                ExpectedHeader {
                    name: "x-gone".to_string(),
                    check: HeaderCheck::Absent,
                },
            ],
        );
        assert!(out.contains(r#"assert!(response.headers().contains_key("set-cookie")"#));
        assert!(out.contains(r#"assert!(!response.headers().contains_key("x-gone")"#));
    }

    #[test]
    fn render_header_assertions_checks_uuid_shape_rather_than_value() {
        let mut out = String::new();
        render_header_assertions(
            &mut out,
            &[ExpectedHeader {
                name: "x-request-id".to_string(),
                check: HeaderCheck::Uuid,
            }],
        );
        assert!(out.contains("header_value.len() == 36"));
        assert!(out.contains("c.is_ascii_hexdigit()"));
        assert!(out.contains("to be a UUID"));
    }

    /// Generated assertion messages are embedded in a Rust string literal, so the header
    /// name must not be quoted inside it — nested double quotes would not compile.
    #[test]
    fn render_header_assertions_messages_contain_no_nested_double_quotes() {
        let mut out = String::new();
        render_header_assertions(
            &mut out,
            &[ExpectedHeader {
                name: "x-total-count".to_string(),
                check: HeaderCheck::Exact("42".to_string()),
            }],
        );
        assert!(!out.contains(r#"header ""#), "message quotes the header name: {out}");
    }

    #[test]
    fn render_handler_headers_emits_the_default_content_type_when_unpinned() {
        let mut out = String::new();
        render_handler_headers(&mut out, &[]);
        assert_eq!(out, "                .header(\"content-type\", \"application/json\")\n");
    }

    #[test]
    fn render_handler_headers_lets_the_fixture_pin_the_content_type() {
        let mut out = String::new();
        render_handler_headers(
            &mut out,
            &[ExpectedHeader {
                name: "content-type".to_string(),
                check: HeaderCheck::Exact("text/html".to_string()),
            }],
        );
        assert!(!out.contains("application/json"));
        assert!(out.contains(r##".header("content-type", r#"text/html"#)"##));
    }

    /// An `<<absent>>` expectation means the handler must not set the header at all.
    #[test]
    fn render_handler_headers_skips_absent_expectations() {
        let mut out = String::new();
        render_handler_headers(
            &mut out,
            &[ExpectedHeader {
                name: "x-gone".to_string(),
                check: HeaderCheck::Absent,
            }],
        );
        assert!(!out.contains("x-gone"));
    }

    #[test]
    fn render_http_test_function_asserts_status_body_and_headers() {
        let fixture = http_fixture(
            expected_response(Some(serde_json::json!({"status": "ok"})), &[("X-Total-Count", "42")]),
            None,
            "GET",
        );
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(out.contains("assert_eq!(response.status_code().as_u16(), 200u16);"));
        assert!(out.contains("assert_eq!(actual_json, expected_json);"));
        assert!(out.contains(r#"response.headers().get("x-total-count")"#));
        assert!(out.contains(r##".header("x-total-count", r#"42"#)"##));
    }

    #[test]
    fn render_http_test_function_omits_body_assertion_when_there_is_no_expected_body() {
        let fixture = http_fixture(expected_response(None, &[]), None, "GET");
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(!out.contains("response.text()"));
        assert!(!out.contains("actual_json"));
    }

    /// A CORS layer answers preflight itself, so the handler's body and headers never
    /// reach the client and must not be asserted.
    #[test]
    fn render_http_test_function_skips_response_assertions_for_cors_preflight() {
        let middleware = HttpMiddleware {
            cors: Some(CorsConfig {
                allow_origins: vec!["https://example.com".to_string()],
                ..CorsConfig::default()
            }),
            ..HttpMiddleware::default()
        };
        let fixture = http_fixture(
            expected_response(Some(serde_json::json!({"status": "ok"})), &[("X-Total-Count", "42")]),
            Some(middleware),
            "OPTIONS",
        );
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");
        assert!(!out.contains("actual_json"));
        assert!(!out.contains("x-total-count"));
    }

    #[test]
    fn render_cors_layer_empty_policy_uses_any() {
        let cors = CorsConfig::default();
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(out.contains("allow_origin(tower_http::cors::Any)"));
        assert!(out.contains("allow_methods(tower_http::cors::Any)"));
        assert!(out.contains("allow_headers(tower_http::cors::Any)"));
    }

    /// An empty CORS policy must not import `HeaderName`/`HeaderValue`/`Method`
    /// — emitting unused imports trips `-D unused_imports` in the consumer.
    #[test]
    fn render_cors_layer_empty_policy_emits_no_axum_http_imports() {
        let cors = CorsConfig::default();
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(!out.contains("use axum::http::"));
    }

    /// `allow_origins` set → `HeaderValue` is referenced, so the import must appear.
    #[test]
    fn render_cors_layer_with_origin_imports_header_value() {
        let cors = CorsConfig {
            allow_origins: vec!["https://example.com".to_string()],
            ..CorsConfig::default()
        };
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(out.contains("use axum::http::HeaderValue;"));
    }

    /// `allow_methods` set → `Method` is referenced.
    #[test]
    fn render_cors_layer_with_method_imports_method() {
        let cors = CorsConfig {
            allow_methods: vec!["GET".to_string()],
            ..CorsConfig::default()
        };
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(out.contains("use axum::http::Method;"));
    }

    /// `allow_headers` containing only prelude-mapped names (content-type, etc.)
    /// must NOT import `HeaderName` — those headers expand to qualified constants.
    #[test]
    fn render_cors_layer_with_only_prelude_headers_omits_header_name() {
        let cors = CorsConfig {
            allow_headers: vec!["content-type".to_string(), "Authorization".to_string()],
            ..CorsConfig::default()
        };
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(!out.contains("HeaderName"));
    }

    /// `allow_headers` containing a custom header → `HeaderName::from_static(...)` is
    /// emitted, so the `HeaderName` import must appear.
    #[test]
    fn render_cors_layer_with_custom_header_imports_header_name() {
        let cors = CorsConfig {
            allow_headers: vec!["X-Custom".to_string()],
            ..CorsConfig::default()
        };
        let mut out = String::new();
        render_cors_layer(&mut out, &cors);
        assert!(out.contains("HeaderName"));
        assert!(out.contains("use axum::http::HeaderName;"));
    }

    /// `#[tokio::test]` builds and drops its own `current_thread` runtime per test; a
    /// pooled HTTP connection created on one such runtime outlives it and is later handed
    /// to a different test's runtime, causing intermittent "error sending request" and
    /// "error decoding response body" failures. Every generated HTTP test must instead run
    /// synchronously and block the shared process-wide runtime from `common::runtime()`.
    #[test]
    fn render_http_test_function_uses_shared_runtime_not_tokio_test() {
        let fixture = http_fixture(expected_response(None, &[]), None, "GET");
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");

        assert!(!out.contains("#[tokio::test]"), "{out}");
        assert!(!out.contains("async fn test_"), "{out}");
        assert!(out.contains("#[test]\nfn test_sample() {\n"), "{out}");
        assert!(out.contains("common::runtime().block_on(async {"), "{out}");
        // The block_on wrapper closes with `});` immediately before the fn's own closing `}`.
        assert!(out.trim_end().ends_with("});\n}"), "{out}");
    }

    /// The static-files middleware path renders through a separate helper
    /// (`render_static_files_test`) with its own closing brace — it must get the same
    /// shared-runtime wrapper as the main HTTP test path.
    #[test]
    fn render_http_test_function_static_files_path_uses_shared_runtime() {
        let middleware = HttpMiddleware {
            static_files: Some(vec![StaticFilesConfig {
                route_prefix: "/public".to_string(),
                files: Vec::new(),
                index_file: false,
                cache_control: None,
            }]),
            ..HttpMiddleware::default()
        };
        let fixture = http_fixture(expected_response(None, &[]), Some(middleware), "GET");
        let mut out = String::new();
        render_http_test_function(&mut out, &fixture, "demo");

        assert!(!out.contains("#[tokio::test]"), "{out}");
        assert!(out.contains("#[test]\nfn test_sample() {\n"), "{out}");
        assert!(out.contains("common::runtime().block_on(async {"), "{out}");
        assert!(out.contains("    });\n}"), "{out}");
    }
}
