//! Verifies the Swift e2e codegen emits `async throws` test method signatures
//! when the call config has `async = true`, and `throws` when `async = false`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "path": "test.txt" }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
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
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn make_group(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture(id)],
    }
}

fn render_swift(toml: &str, fixture_id: &str) -> Vec<alef::core::backend::GeneratedFile> {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

fn smoke_test_content(files: &[alef::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
module = "DemoClient"
result_var = "result"

[[crates.e2e.call.args]]
name = "path"
field = "input.path"
arg_type = "file_path"
"#;

#[test]
fn test_async_true_emits_async_throws() {
    let mut toml = BASE_TOML.to_string();
    // Insert "async = true\n" before the closing [[ crates.e2e.call.args ]] section
    toml.insert_str(toml.rfind("[[crates.e2e.call.args]]").unwrap(), "async = true\n\n");
    let files = render_swift(&toml, "async_test");
    let content = smoke_test_content(&files);
    assert!(
        content.contains("func testAsyncTest() async throws {"),
        "Expected 'async throws' when call config has async = true,\ngot:\n{}",
        content
    );
}

#[test]
fn test_async_false_emits_throws_only() {
    let mut toml = BASE_TOML.to_string();
    toml.insert_str(toml.rfind("[[crates.e2e.call.args]]").unwrap(), "async = false\n\n");
    let files = render_swift(&toml, "sync_test");
    let content = smoke_test_content(&files);
    assert!(
        content.contains("func testSyncTest() throws {"),
        "Expected 'throws' (no async) when call config has async = false,\ngot:\n{}",
        content
    );
    assert!(
        !content.contains("async throws"),
        "Should not contain 'async throws' when async = false"
    );
}

#[test]
fn test_async_defaults_to_false() {
    // When async is not specified, it should default to false.
    let toml = BASE_TOML.to_string(); // No async line
    let files = render_swift(&toml, "default_test");
    let content = smoke_test_content(&files);
    assert!(
        content.contains("func testDefaultTest() throws {"),
        "Expected 'throws' (no async) when async is not specified (defaults to false),\ngot:\n{}",
        content
    );
}
