//! Verifies that the generated mock-server binary and embedded mock_server
//! module honor fixture-declared response headers.
//!
//! Both shapes are checked:
//!   * `MockResponse.headers` (demo-client `mock_response.headers`)
//!   * `HttpExpectedResponse.headers` (consumer `http.expected_response.headers`)
//!
//! The generated source is inspected for the iteration code that applies each
//! header to the axum `Response::builder()`. This guards against regressions
//! where the mock-server silently drops fixture-declared headers (causing
//! consumers' header assertions to fail with `null`).

use alef::e2e::codegen::rust::{render_mock_server_binary, render_mock_server_module};

#[test]
fn mock_server_module_route_struct_has_headers_field() {
    let module = render_mock_server_module();
    assert!(
        module.contains("pub headers: Vec<(String, String)>"),
        "MockRoute struct must expose a `headers: Vec<(String, String)>` field so test setup \
         code can pass fixture headers through to the response"
    );
}

#[test]
fn mock_server_module_applies_headers_to_response() {
    let module = render_mock_server_module();
    assert!(
        module.contains("for (name, value) in &route.headers"),
        "handle_request must iterate `route.headers` and apply each entry via \
         `builder.header(name, value)` — otherwise fixture headers are dropped"
    );
    assert!(
        module.contains("builder = builder.header(name, value)"),
        "handle_request must apply headers via `builder.header(name, value)`"
    );
}

#[test]
fn mock_server_binary_route_struct_has_headers_field() {
    let bin = render_mock_server_binary();
    assert!(
        bin.contains("headers: Vec<(String, String)>"),
        "Mock-server binary's MockRoute must include a `headers` field"
    );
}

#[test]
fn mock_server_binary_deserializes_headers_from_both_schemas() {
    let bin = render_mock_server_binary();
    assert!(
        bin.contains("struct MockResponse") && bin.contains("headers: HashMap<String, String>"),
        "Mock-server binary must deserialize `mock_response.headers`"
    );
    assert!(
        bin.contains("struct HttpExpectedResponse"),
        "Mock-server binary must define HttpExpectedResponse"
    );
    assert!(
        bin.contains("headers: mock.headers.clone()")
            && bin.contains("headers: http.expected_response.headers.clone()"),
        "as_mock_response() must bridge headers from both schemas to the unified MockResponse"
    );
}

#[test]
fn mock_server_binary_applies_headers_to_response() {
    let bin = render_mock_server_binary();
    assert!(
        bin.contains("for (name, value) in &route.headers"),
        "Mock-server binary's serve_route must iterate `route.headers` and apply each entry — \
         otherwise consumer fixtures' Access-Control-*, X-Request-Id, WWW-Authenticate, etc. \
         headers come back as null on the consumer side"
    );
}
