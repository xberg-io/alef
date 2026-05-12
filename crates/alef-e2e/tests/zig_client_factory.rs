//! Verifies the Zig e2e codegen emits client-object instantiation when
//! `CallOverride.client_factory` is set, and falls back to flat function calls
//! when it is absent.
//!
//! The test mirrors the go codegen pattern: when `client_factory` is set, the
//! generated test must create a client via the factory, call methods on it, and
//! clean up. When absent, the module function is called directly — the kreuzberg
//! flat-function style must remain untouched.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::zig::ZigE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "request": { "model": "gpt-4o", "messages": [] } }),
        mock_response: Some(alef_e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::HashMap::new(),
        }),
        visitor: None,
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

fn render_zig_smoke(toml: &str, fixture_id: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.zig"))
        .expect("smoke_test.zig is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "literllm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "liter_llm"
result_var = "result"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// When `client_factory` is set, the generated test must:
///   1. create a client via the named factory function
///   2. call the method on the client instance (_client.chat)
///   3. NOT call the module-level function directly (liter_llm.chat)
#[test]
fn with_client_factory_emits_client_instantiation() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.zig]
client_factory = "create_client"
result_is_json_struct = true
"#,
        BASE_TOML
    );
    let rendered = render_zig_smoke(&toml, "smoke_basic");

    assert!(
        rendered.contains("create_client("),
        "must call create_client factory. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("_client.chat("),
        "must call chat on client instance. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("liter_llm.chat("),
        "must NOT call liter_llm.chat directly when client_factory is set. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("MOCK_SERVER_URL"),
        "must include MOCK_SERVER_URL in mock url construction. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("_client.free(") || rendered.contains("defer"),
        "must clean up client. Rendered:\n{rendered}"
    );
}

/// When `client_factory` is absent, the generator must fall back to the flat
/// module-function call pattern (kreuzberg style). This ensures no regression.
#[test]
fn without_client_factory_emits_flat_function_call() {
    let rendered = render_zig_smoke(BASE_TOML, "smoke_basic");

    assert!(
        rendered.contains("liter_llm.chat("),
        "must call module-level function directly. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("create_client("),
        "must NOT call create_client when client_factory is absent. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("_client."),
        "must NOT reference client instance when client_factory is absent. Rendered:\n{rendered}"
    );
}
