//! Verifies the Dart e2e codegen handles `json_object` args with a scalar
//! `element_type` (e.g. `"String"`). Regression test for a bug where the
//! match on `element_type` only handled `"BatchBytesItem"`/`"BatchFileItem"`
//! and silently dropped scalar-typed array args (notably `texts: List<String>`
//! for `embed_texts*`), causing the generated Dart test to crash.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::dart::DartE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("embed".to_string()),
        description: "embed texts test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input,
        mock_response: None,
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
        source: "embed.json".to_string(),
        http: None,
    }
}

fn make_group(id: &str, input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "embed".to_string(),
        fixtures: vec![make_fixture(id, input)],
    }
}

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "kreuzberg"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "kreuzberg"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "embed_texts"
result_var = "result"

[[crates.e2e.call.args]]
name = "texts"
field = "input.texts"
type = "json_object"
element_type = "String"
"#;

fn render(fixture_id: &str, input: serde_json::Value) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id, input)];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("embed_test.dart"))
        .expect("embed_test.dart is emitted")
        .content
        .clone()
}

/// `element_type = "String"` with an array value must emit a typed Dart
/// list literal `<String>['a', 'b']` passed as the named argument
/// matching the camelCased arg name. Regression test for the bug where
/// non-Batch element types were silently dropped from the call.
#[test]
fn json_object_arg_with_string_element_type_emits_typed_list_literal() {
    let rendered = render(
        "embed_texts_async_happy",
        serde_json::json!({ "texts": ["First", "Second"] }),
    );

    assert!(
        rendered.contains("<String>['First', 'Second']"),
        "must emit typed `<String>[...]` list literal for the texts arg. Rendered:\n{rendered}"
    );
    // Sanity check: the call must include the embedTexts invocation.
    assert!(
        rendered.contains("embedTexts("),
        "must emit embedTexts( call. Rendered:\n{rendered}"
    );
}
