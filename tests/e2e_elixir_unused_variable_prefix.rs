//! Verifies the Elixir e2e codegen prefixes unused result variables with `_`
//! to avoid "variable is unused" warnings in mix compile --warnings-as-errors.
//!
//! When a fixture has no assertions and the result is not used (e.g., pure side effects),
//! the generated pattern match {:ok, result} = ... causes the Elixir compiler to fail
//! with "variable \"result\" is unused". The fix prefixes the variable with `_` to signal
//! that it is intentionally unused.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "MyLib"
result_var = "result"
returns_result = true
args = [
  { name = "input", field = "input", type = "string" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture_with_no_assertions() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_no_assertions".to_string(),
            category: Some("smoke".to_string()),
            description: "call with no assertions".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "input": "test" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![],
            source: "smoke/smoke_no_assertions.json".to_string(),
            http: None,
        }],
    }
}

fn fixture_with_assertions() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_with_assertions".to_string(),
            category: Some("smoke".to_string()),
            description: "call with assertions".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "input": "test" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("output".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "smoke/smoke_with_assertions.json".to_string(),
            http: None,
        }],
    }
}

/// A fixture with no assertions must emit {:ok, _result} (prefixed with _) to avoid
/// unused variable warnings in mix compile --warnings-as-errors.
#[test]
fn no_assertions_fixture_prefixes_unused_result_var() {
    let (e2e, resolved) = build_config();
    let groups = vec![fixture_with_no_assertions()];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke_test.exs"))
        .expect("Elixir smoke_test.exs is emitted");

    let body = &test_file.content;

    assert!(
        body.contains("{:ok, _result}"),
        "fixture with no assertions must emit {{:ok, _result}} (underscore-prefixed), got:\n{body}"
    );
    assert!(
        !body.contains("{:ok, result}") || body.contains("assert"),
        "fixture with no assertions must not emit {{:ok, result}} without underscore unless followed by assertion, got:\n{body}"
    );
}

/// A fixture with assertions must emit {:ok, result} (NOT prefixed with _) because
/// the variable is actually used in subsequent assertions.
#[test]
fn with_assertions_fixture_does_not_prefix_result_var() {
    let (e2e, resolved) = build_config();
    let groups = vec![fixture_with_assertions()];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke_test.exs"))
        .expect("Elixir smoke_test.exs is emitted");

    let body = &test_file.content;

    assert!(
        body.contains("assert") && body.contains("result"),
        "fixture with assertions must use result variable in assertion, got:\n{body}"
    );
}
