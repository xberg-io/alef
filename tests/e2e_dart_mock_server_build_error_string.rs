//! Verifies that Dart e2e codegen correctly emits Dart string interpolation
//! in the mock-server build error message without over-escaping braces.
//!
//! Originally a regression test for the error message being emitted as
//! `'mock-server build failed: \${_build.stderr}'` with double braces in the
//! per-fixture `*_test.dart` file. `fix(e2e): spawn the dart mock server
//! through a generated shared helper (#306)` removed that per-file inline
//! generation entirely and moved the build/spawn logic into the shared,
//! alef-generated `e2e_helpers.dart` (`project::render_e2e_helpers`), which
//! emits the message as a plain raw-string literal (no `format!`/`write!`
//! escaping involved) using a `build` local, not `_build`. This test now
//! checks that file for the same thing: correct, un-escaped interpolation. ~keep

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
use std::collections::BTreeMap;

fn make_http_fixture(id: &str, description: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
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
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("e2e_helpers.dart"))
        .expect("e2e_helpers.dart is emitted for a crate with a standalone-mock-server fixture")
        .content
        .clone()
}

/// Verify that the mock-server build error message contains proper Dart
/// string interpolation: `${build.stderr}` with single braces, not double.
#[test]
fn mock_server_build_error_string_interpolation_correct() {
    let fixtures = vec![make_http_fixture(
        "http_test_fixture",
        "HTTP test that triggers mock-server build",
    )];

    let rendered = render(fixtures);

    assert!(
        rendered.contains("mock-server build failed: ${build.stderr}"),
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
        rendered.contains("throw StateError('mock-server build failed: ${build.stderr}')"),
        "full error statement must be present with correct interpolation. Rendered:\n{rendered}"
    );
}
