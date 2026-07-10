//! Verifies the Dart e2e codegen handles nullable string assertions correctly.
//! Regression test for a bug where string equals assertions on simple nullable
//! result types (e.g. Option<String> → String?) would emit `.toString().trim()`
//! on a null value, causing a runtime NoSuchMethodError at test time.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str, description: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("language-detection".to_string()),
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
        assertions,
        source: "language_detection.json".to_string(),
        http: None,
    }
}

fn make_group(fixtures: Vec<Fixture>) -> FixtureGroup {
    FixtureGroup {
        category: "language-detection".to_string(),
        fixtures,
    }
}

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-language-pack"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_language_pack"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "detect_language_from_content"
result_var = "result"
result_is_simple = true

[[crates.e2e.call.args]]
name = "content"
field = "input.content"
type = "string"
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
        .find(|f| f.path.to_string_lossy().contains("language_detection_test.dart"))
        .expect("language_detection_test.dart is emitted")
        .content
        .clone()
}

/// When a result_is_simple string assertion compares against an expected value,
/// the generated code must use null-coalescing (?? '') to handle nullable String?
/// results. This prevents NoSuchMethodError at runtime when the result is null.
#[test]
fn nullable_string_equals_assertion_uses_null_coalescing() {
    let fixtures = vec![make_fixture(
        "detect_language_simple_nullable_string",
        "detect_language_from_content recognizes #!/bin/bash shebang",
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::Value::String("bash".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    )];

    let rendered = render(fixtures);

    // Expected pattern: `expect((result ?? '').toString().trim(), equals('bash'.toString().trim()));`
    assert!(
        rendered.contains("(result ?? '')"),
        "must use null-coalescing operator (?? '') for nullable string assertion. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("expect(") && rendered.contains("equals("),
        "must emit an equals assertion. Rendered:\n{rendered}"
    );
}

/// Verify not_equals assertions also use null-coalescing for nullable strings.
#[test]
fn nullable_string_not_equals_assertion_uses_null_coalescing() {
    let fixtures = vec![make_fixture(
        "detect_language_not_equal_nullable",
        "detect_language from content is not unknown",
        vec![Assertion {
            assertion_type: "not_equals".to_string(),
            field: None,
            value: Some(serde_json::Value::String("unknown".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    )];

    let rendered = render(fixtures);

    assert!(
        rendered.contains("(result ?? '')"),
        "must use null-coalescing operator (?? '') for nullable string not_equals assertion. Rendered:\n{rendered}"
    );
}
