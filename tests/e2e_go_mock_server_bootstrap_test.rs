//! Tests for Go e2e TestMain mock-server bootstrap logic.
//!
//! These tests verify that when mock_server fixtures are present, the generated
//! TestMain includes the bootstrap logic to spawn the mock-server binary in
//! standalone mode (when MOCK_SERVER_URL is not already set).

use alef::core::config::new_config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::fixture::{
    Assertion, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest, MockResponse,
};
use std::collections::BTreeMap;

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["go"]

[[crates]]
name = "testlib"
sources = ["src/lib.rs"]

[crates.go]
module = "github.com/test/testlib"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "testlib"
result_var = "result"
returns_result = true
args = []
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn make_mock_server_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "mock_server_tests".to_string(),
        fixtures: vec![Fixture {
            id: "test_with_mock_response".to_string(),
            category: Some("mock_server_tests".to_string()),
            description: "Test with mock server response".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({}),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::json!({"status": "ok"})),
                stream_chunks: None,
                headers: BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/mock_response.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn test_go_main_test_with_mock_server_fixture() {
    let (e2e_config, resolved_config) = build_config();
    let groups = vec![make_mock_server_fixture()];

    let files = GoCodegen
        .generate(&groups, &e2e_config, &resolved_config, &[], &[])
        .expect("generation succeeds");

    let main_test_file = files
        .iter()
        .find(|f| f.path.ends_with("main_test.go"))
        .expect("main_test.go is generated");

    let content = &main_test_file.content;

    assert!(
        content.contains("os.Getenv(\"MOCK_SERVER_URL\")"),
        "TestMain should check MOCK_SERVER_URL"
    );
    assert!(
        content.contains("mock-server"),
        "TestMain should reference mock-server binary"
    );
    assert!(
        content.contains("fixtures"),
        "TestMain should reference fixtures directory"
    );
    assert!(
        content.contains("MOCK_SERVER_URL="),
        "TestMain should parse MOCK_SERVER_URL from mock-server stdout"
    );
    assert!(content.contains("bufio"), "TestMain should import bufio for scanner");
    assert!(
        content.contains("strings"),
        "TestMain should import strings for prefix matching"
    );

    assert!(
        content.contains("cargo"),
        "TestMain should use cargo to build mock-server if missing"
    );

    assert!(
        content.contains("encoding/json"),
        "TestMain should import JSON parsing helpers for MOCK_SERVERS"
    );
    assert!(
        content.contains("json.Unmarshal([]byte(payload), &servers)"),
        "TestMain should parse MOCK_SERVERS metadata"
    );
}

#[test]
fn test_go_main_test_fixture_has_http_fixtures_not_mock_server() {
    let (e2e_config, resolved_config) = build_config();

    let groups = vec![FixtureGroup {
        category: "http_tests".to_string(),
        fixtures: vec![Fixture {
            id: "test_http".to_string(),
            category: Some("http_tests".to_string()),
            description: "HTTP test".to_string(),
            tags: vec![],
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
            source: "test/http.json".to_string(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/test".to_string(),
                    method: "GET".to_string(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    path: "/test".to_string(),
                    method: "GET".to_string(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: None,
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
        }],
    }];

    let files = GoCodegen
        .generate(&groups, &e2e_config, &resolved_config, &[], &[])
        .expect("generation succeeds");

    let main_test_file = files
        .iter()
        .find(|f| f.path.ends_with("main_test.go"))
        .expect("main_test.go is generated");

    let content = &main_test_file.content;

    assert!(
        content.contains("SUT_URL"),
        "TestMain for HTTP fixtures should use SUT_URL"
    );
    assert!(
        content.contains("harness"),
        "TestMain for HTTP fixtures should reference harness"
    );
    assert!(
        !content.contains("Spawn mock-server binary if MOCK_SERVER_URL"),
        "TestMain for HTTP-only fixtures should not have mock-server bootstrap logic"
    );
}
