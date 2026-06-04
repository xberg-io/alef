//! Verifies the C e2e Makefile generator emits both `smoke:` and `test:` targets
//! with mock-server orchestration parameterized via a `define`/`endef` macro.
//!
//! The `smoke` target runs `./$(TARGET) --smoke` while `test` runs `./$(TARGET)`
//! for the full suite. Both targets wrap the same mock-server build/spawn/cleanup
//! logic via a shared `run_with_mock_server` macro that receives TEST_CMD as a
//! variable override.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::c::CCodegen;
use alef::e2e::fixture::{
    Assertion, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest, MockResponse,
};
use std::collections::BTreeMap;

fn build_config_with_mock() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "sample"
result_var = "result"
args = [
  { name = "messages", field = "messages", type = "string" },
]

[crates.e2e.call.overrides.c]
header = "sample_llm.h"
function = "sample_chat"
prefix = "sample"
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_fixture_with_http() -> FixtureGroup {
    FixtureGroup {
        category: "chat".to_string(),
        fixtures: vec![Fixture {
            id: "chat_basic".to_string(),
            category: Some("chat".to_string()),
            description: "basic chat".to_string(),
            tags: vec!["smoke".to_string()],
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "messages": "hello" }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::json!({ "content": "hi there" })),
                stream_chunks: None,
                headers: BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/v1/chat/completions".to_string(),
                    method: "POST".to_string(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "POST".to_string(),
                    path: "/v1/chat/completions".to_string(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: Some(serde_json::json!({ "messages": "hello" })),
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: Some(serde_json::json!({ "content": "hi there" })),
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
        }],
    }
}

#[test]
fn c_makefile_emits_smoke_and_test_targets() {
    let cfg = build_config_with_mock();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![build_fixture_with_http()];
    let files = CCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("C generation succeeds");

    let makefile = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Makefile"))
        .expect("Makefile should be emitted");

    let content = &makefile.content;

    // Verify .PHONY declaration includes both test and smoke
    assert!(
        content.contains(".PHONY: all clean test smoke"),
        ".PHONY must declare both test and smoke targets. Got:\n{content}"
    );

    // Verify smoke target is present and runs with --smoke flag
    assert!(
        content.contains("smoke: $(TARGET)"),
        "smoke target must be defined. Got:\n{content}"
    );
    assert!(
        content.contains("./$(TARGET) --smoke"),
        "smoke target must run test binary with --smoke flag. Got:\n{content}"
    );

    // Verify test target is present
    assert!(
        content.contains("test: $(TARGET)"),
        "test target must be defined. Got:\n{content}"
    );

    // Verify mock-server orchestration macro is defined
    assert!(
        content.contains("define run_with_mock_server"),
        "run_with_mock_server macro must be defined. Got:\n{content}"
    );
    assert!(
        content.contains("endef"),
        "run_with_mock_server macro must be closed with endef. Got:\n{content}"
    );

    // Verify the macro uses TEST_CMD variable
    assert!(
        content.contains("$(TEST_CMD)"),
        "run_with_mock_server macro must reference $(TEST_CMD) variable. Got:\n{content}"
    );

    // Verify both test and smoke invoke the macro with different TEST_CMD values
    assert!(
        content.contains("@TEST_CMD='./$(TARGET)' $(MAKE) -s run_with_mock_server"),
        "test target must invoke run_with_mock_server with full test command. Got:\n{content}"
    );
    assert!(
        content.contains("@TEST_CMD='./$(TARGET) --smoke' $(MAKE) -s run_with_mock_server"),
        "smoke target must invoke run_with_mock_server with --smoke flag. Got:\n{content}"
    );

    // Verify the run_with_mock_server target delegates to the macro
    assert!(
        content.contains("run_with_mock_server:"),
        "run_with_mock_server target must be defined. Got:\n{content}"
    );
    assert!(
        content.contains("$(run_with_mock_server)"),
        "run_with_mock_server target must invoke the macro. Got:\n{content}"
    );
}

#[test]
fn c_makefile_without_mock_server_has_simple_smoke_and_test() {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-markdown-rs"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "htm"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]

[crates.e2e.call.overrides.c]
header = "sample_markdown.h"
function = "htm_convert"
prefix = "htm"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");

    // Fixture with no HTTP requirements (no mock server needed)
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "basic conversion".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "html": "<p>hi</p>" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }];

    let files = CCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("C generation succeeds");

    let makefile = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Makefile"))
        .expect("Makefile should be emitted");

    let content = &makefile.content;

    // Without mock server, both smoke and test should be simple direct invocations
    assert!(
        content.contains(".PHONY: all clean test smoke"),
        ".PHONY must declare both test and smoke targets. Got:\n{content}"
    );

    assert!(content.contains("test: $(TARGET)"), "test target must be defined");
    assert!(content.contains("smoke: $(TARGET)"), "smoke target must be defined");

    // Simple targets without mock server orchestration
    let test_target_lines: Vec<&str> = content
        .lines()
        .skip_while(|l| !l.starts_with("test: "))
        .take_while(|l| !l.starts_with("smoke:"))
        .collect();
    let test_section = test_target_lines.join("\n");
    assert!(
        test_section.contains("./$(TARGET)"),
        "test target must directly run ./$(TARGET)"
    );

    let smoke_target_lines: Vec<&str> = content
        .lines()
        .skip_while(|l| !l.starts_with("smoke: "))
        .take_while(|l| !l.starts_with("run_with") && !l.starts_with("clean:"))
        .collect();
    let smoke_section = smoke_target_lines.join("\n");
    assert!(
        smoke_section.contains("./$(TARGET) --smoke"),
        "smoke target must run ./$(TARGET) --smoke"
    );

    // No macro should be emitted when mock server is not needed
    assert!(
        !content.contains("define run_with_mock_server"),
        "run_with_mock_server macro must not be emitted when mock server is not needed"
    );
}
