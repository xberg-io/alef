//! Verifies that the Swift e2e codegen derives the config-from-json helper name
//! from configured or inferred type metadata, not from a hardcoded downstream name.
//!
//! For example, with `options_type = "ProcessConfig"`, it should emit
//! `processConfigFromJson(...)`.

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
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({
            "request": { "model": "gpt-4o", "messages": [] },
            "config": { "timeout_ms": 5000 }
        }),
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
name = "sample-language-pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "parse"
module = "sample_language_pack"
result_var = "result"
async = true

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "json_object"
"#;

/// When `options_type = "ProcessConfig"` is set in CallOverride.swift, the codegen
/// must emit `processConfigFromJson(...)` instead of hardcoded `extractionConfigFromJson(...)`.
#[test]
fn config_from_json_uses_options_type_name() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
options_type = "ProcessConfig"
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_with_config");
    let rendered = smoke_test_content(&files);

    assert!(
        rendered.contains("processConfigFromJson("),
        "must emit processConfigFromJson when options_type = ProcessConfig. Rendered:\n{rendered}"
    );

    assert!(
        !rendered.contains("extractionConfigFromJson("),
        "must NOT hardcode extractionConfigFromJson when options_type is set. Rendered:\n{rendered}"
    );
}

/// Call-level `options_type` is binding-agnostic type metadata and should be used
/// when the Swift override does not provide a more specific wrapper type.
#[test]
fn config_from_json_uses_call_level_options_type_name() {
    let base = BASE_TOML.replacen(
        "[[crates.e2e.call.args]]",
        "options_type = \"ExampleConfig\"\n\n[[crates.e2e.call.args]]",
        1,
    );
    let toml = format!(
        r#"{base}
[crates.e2e.call.overrides.swift]
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_with_config");
    let rendered = smoke_test_content(&files);

    assert!(
        rendered.contains("exampleConfigFromJson("),
        "must emit exampleConfigFromJson when call-level options_type = ExampleConfig. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("extractionConfigFromJson("),
        "must NOT hardcode extractionConfigFromJson when Swift override options_type is absent. Rendered:\n{rendered}"
    );
}

/// With no explicit type metadata, derive a neutral helper from the actual arg name.
#[test]
fn config_from_json_without_options_type_uses_arg_name() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.swift]
options_via = "from_json"
"#
    );
    let files = render_swift(&toml, "smoke_with_config");
    let rendered = smoke_test_content(&files);

    assert!(
        rendered.contains("configFromJson("),
        "must derive configFromJson from arg name when options_type is absent. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("extractionConfigFromJson("),
        "must NOT hardcode extractionConfigFromJson when options_type is absent. Rendered:\n{rendered}"
    );
}
