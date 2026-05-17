//! HTTP integration test generation for Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::escape::rust_raw_string;
use crate::fixture::{CorsConfig, Fixture, StaticFilesConfig};

/// How to call a method on axum_test::TestServer in generated code.
enum ServerCall<'a> {
    /// Emit `server.get(path)` / `server.post(path)` etc.
    Shorthand(&'a str),
    /// Emit `server.method(axum::http::Method::OPTIONS, path)` etc.
    AxumMethod(&'a str),
}

/// How to register a route on a spikard App in generated code.
enum RouteRegistration<'a> {
    /// Emit `spikard::get(path)` / `spikard::post(path)` etc.
    Shorthand(&'a str),
    /// Emit `spikard::RouteBuilder::new(spikard::Method::Options, path)` etc.
    Explicit(&'a str),
}

/// Generate a complete integration test function for an http fixture.
///
/// Builds a real spikard `App` with a handler that returns the expected
/// response, then uses `axum_test::TestServer` to send the request and
/// assert the status code.
pub fn render_http_test_function(out: &mut String, fixture: &Fixture, dep_name: &str) {
    let http = match &fixture.http {
        Some(h) => h,
        None => return,
    };

    let fn_name = crate::escape::sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let route = &http.handler.route;

    // spikard provides convenience functions for GET/POST/PUT/PATCH/DELETE.
    // All other methods (HEAD, OPTIONS, TRACE, etc.) must use RouteBuilder::new directly.
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

    // Serialize expected response body (if any).
    let body_str = match &http.expected_response.body {
        Some(b) => serde_json::to_string(b).unwrap_or_else(|_| "{}".to_string()),
        None => String::new(),
    };
    let body_literal = rust_raw_string(&body_str);

    // Serialize request body (if any).
    let req_body_str = match &http.request.body {
        Some(b) => serde_json::to_string(b).unwrap_or_else(|_| "{}".to_string()),
        None => String::new(),
    };
    let has_req_body = !req_body_str.is_empty();

    // Extract middleware from handler (if any).
    let middleware = http.handler.middleware.as_ref();
    let cors_cfg: Option<&CorsConfig> = middleware.and_then(|m| m.cors.as_ref());
    let static_files_cfgs: Option<&Vec<StaticFilesConfig>> = middleware.and_then(|m| m.static_files.as_ref());
    let has_static_files = static_files_cfgs.is_some_and(|v| !v.is_empty());

    let _ = writeln!(out, "#[tokio::test]");
    let _ = writeln!(out, "async fn test_{fn_name}() {{");
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
    let _ = writeln!(out, "                .header(\"content-type\", \"application/json\")");
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
    for (name, value) in &http.request.headers {
        let n = rust_raw_string(name);
        let v = rust_raw_string(value);
        let _ = writeln!(out, "        .add_header({n}, {v})");
    }

    // Add request body if present (pass as a JSON string so axum-test's bytes() API gets a Bytes value).
    if has_req_body {
        let req_body_literal = rust_raw_string(&req_body_str);
        let _ = writeln!(
            out,
            "        .bytes(bytes::Bytes::copy_from_slice({req_body_literal}.as_bytes()))"
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
/// `tower_http::services::ServeDir`, bypassing the spikard App entirely.
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
    let _ = writeln!(out, "}}");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
