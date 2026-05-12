//! Verifies the Dart e2e codegen emits client-object instantiation when
//! `CallOverride.client_factory` is set, and falls back to static bridge-class
//! calls when absent (kreuzberg flat-function style).

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::dart::DartE2eCodegen;
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

fn render_dart_smoke(toml: &str, fixture_id: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.dart"))
        .expect("smoke_test.dart is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "liter_llm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
result_var = "result"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// When `client_factory` is set, the generated test must:
///   1. derive the mock URL for the fixture
///   2. create a client via the named factory (camelCased)
///   3. call the method on the client instance (_client.chat)
///   4. NOT use the static bridge-class call directly
#[test]
fn with_client_factory_emits_client_instantiation() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.dart]
client_factory = "create_client"
"#,
        BASE_TOML
    );
    let rendered = render_dart_smoke(&toml, "smoke_basic");

    assert!(
        rendered.contains("createClient("),
        "must call createClient factory (camelCased). Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("_client.chat("),
        "must call chat on client instance. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("LiterLlm.chat(") && !rendered.contains("LiterLlmBridge.chat("),
        "must NOT call static bridge-class method when client_factory is set. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("MOCK_SERVER_URL"),
        "must include MOCK_SERVER_URL in mock url construction. Rendered:\n{rendered}"
    );
}

/// When `client_factory` is absent, the generator must fall back to the static
/// bridge-class call pattern (kreuzberg style). This ensures no regression.
#[test]
fn without_client_factory_emits_static_bridge_call() {
    let rendered = render_dart_smoke(BASE_TOML, "smoke_basic");

    // The bridge class name is derived from pubspec_name → PascalCase → "LiterLlm"
    assert!(
        rendered.contains(".chat("),
        "must emit a .chat( call. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("createClient("),
        "must NOT call createClient when client_factory is absent. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("_client."),
        "must NOT reference _client when client_factory is absent. Rendered:\n{rendered}"
    );
}
