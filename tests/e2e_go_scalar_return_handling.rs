//! Tests for Go e2e generator handling of scalar return types (bool, uint, *string, []string)
//! that don't return errors.
//!
//! These tests verify that functions with `returns_result = false` and `result_is_simple = true`
//! (or `result_is_array = true`) are correctly emitted as value-returning functions, not error-handling functions.

use alef::core::config::new_config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

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

[crates.e2e.calls.has_language]
function = "has_language"
module = "testlib"
result_var = "result"
result_is_simple = true
args = [{ name = "name", field = "language", type = "string" }]

[crates.e2e.calls.has_language.overrides.go]
returns_result = false
result_is_pointer = false

[crates.e2e.calls.language_count]
function = "language_count"
module = "testlib"
result_var = "result"
result_is_simple = true
args = []

[crates.e2e.calls.language_count.overrides.go]
returns_result = false
result_is_pointer = false
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn make_bool_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            id: "test_has_language_bool".to_string(),
            category: Some("test".to_string()),
            description: "Test bool return handling".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("has_language".to_string()),
            input: serde_json::json!({"language": "python"}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "is_true".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/test_has_language_bool.json".to_string(),
            http: None,
        }],
    }
}

fn make_uint_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            id: "test_language_count".to_string(),
            category: Some("test".to_string()),
            description: "Test uint return handling".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("language_count".to_string()),
            input: serde_json::json!({}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "greater_than_or_equal".to_string(),
                field: None,
                value: Some(serde_json::json!(1)),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/test_language_count.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn test_go_bool_return_not_treated_as_error() {
    let (e2e_config, config) = build_config();
    let groups = vec![make_bool_fixture()];

    let codegen = GoCodegen;
    let generated = codegen
        .generate(&groups, &e2e_config, &config, &[], &[])
        .expect("code generation should succeed");

    let test_file = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("_test.go") && f.content.contains("HasLanguage"))
        .expect("a _test.go file exercising HasLanguage should be generated");

    let content = &test_file.content;

    assert!(
        !content.contains("err := pkg.HasLanguage"),
        "Boolean return should not be assigned to `err` variable. Generated code:\n{}",
        content
    );

    let has_error_check = content.contains("if err != nil") && content.contains("HasLanguage");
    assert!(
        !has_error_check,
        "Boolean return should not be checked for error. Generated code:\n{}",
        content
    );
}

#[test]
fn test_go_uint_return_not_treated_as_error() {
    let (e2e_config, config) = build_config();
    let groups = vec![make_uint_fixture()];

    let codegen = GoCodegen;
    let generated = codegen
        .generate(&groups, &e2e_config, &config, &[], &[])
        .expect("code generation should succeed");

    let test_file = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("_test.go") && f.content.contains("LanguageCount"))
        .expect("a _test.go file exercising LanguageCount should be generated");

    let content = &test_file.content;

    assert!(
        !content.contains("err := pkg.LanguageCount"),
        "Uint return should not be assigned to `err` variable. Generated code:\n{}",
        content
    );

    let has_error_check = content.contains("if err != nil") && content.contains("LanguageCount");
    assert!(
        !has_error_check,
        "Uint return should not be checked for error. Generated code:\n{}",
        content
    );
}
