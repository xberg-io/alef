//! Verifies that Dart e2e codegen correctly emits Dart string interpolation
//! in the mock-server build error message without over-escaping braces.
//!
//! Regression test: The error message was emitted as
//! `'mock-server build failed: \${_build.stderr}'` with double braces,
//! which Dart interpreted as a literal string instead of interpolating
//! `${_build.stderr}`. The fix removes the unnecessary escaping so the
//! output Dart source contains `${_build.stderr}` exactly.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
use std::collections::BTreeMap;

fn make_http_fixture(id: &str, description: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("http".to_string()),
        description: description.to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![],
        source: "http.json".to_string(),
        http: Some(HttpFixture {
            handler: HttpHandler {
                route: "/test".to_string(),
                method: "GET".to_string(),
                body_schema: None,
                parameters: BTreeMap::new(),
                middleware: None,
            },
            request: HttpRequest {
                method: "GET".to_string(),
                path: "/test".to_string(),
                headers: BTreeMap::new(),
                query_params: BTreeMap::new(),
                cookies: BTreeMap::new(),
                body: None,
                form_data: None,
                content_type: None,
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: Some(serde_json::json!({"ok": true})),
                headers: BTreeMap::new(),
                body_partial: None,
                validation_errors: Some(Vec::new()),
            },
        }),
    }
}

fn make_group(fixtures: Vec<Fixture>) -> FixtureGroup {
    FixtureGroup {
        category: "http".to_string(),
        fixtures,
    }
}

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-app"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_app"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "fetch_data"
"#;

fn render(fixtures: Vec<Fixture>) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixtures)];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("http_test.dart"))
        .expect("http_test.dart is emitted")
        .content
        .clone()
}

/// Verify that the mock-server build error message contains proper Dart
/// string interpolation: `${_build.stderr}` with single braces, not double.
#[test]
fn mock_server_build_error_string_interpolation_correct() {
    let fixtures = vec![make_http_fixture(
        "http_test_fixture",
        "HTTP test that triggers mock-server build",
    )];

    let rendered = render(fixtures);

    assert!(
        rendered.contains("mock-server build failed: ${_build.stderr}"),
        "mock-server build error must contain proper Dart string interpolation. Rendered:\n{rendered}"
    );

    assert!(
        !rendered.contains("mock-server build failed: \\${"),
        "error message must not have escaped $ (backslash before $). Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("mock-server build failed: ${{"),
        "error message must not have double braces {{ (Dart needs single ${{). Rendered:\n{rendered}"
    );

    assert!(
        rendered.contains("throw StateError('mock-server build failed: ${_build.stderr}')"),
        "full error statement must be present with correct interpolation. Rendered:\n{rendered}"
    );
}
