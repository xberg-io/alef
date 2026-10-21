//! Verifies the Elixir e2e codegen preserves the declared `error` assertion value
//! (`Assertion.value`) instead of dropping it in favor of a bare `{:error, _}` match,
//! and that the validation-category (`resolved_category() == "validation"`) shape
//! actually exercises the operation under test rather than stopping after asserting
//! that engine/handle creation failed.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn error_assertion(value: Option<&str>) -> Assertion {
    Assertion {
        skip: None,
        assertion_type: "error".to_string(),
        field: None,
        value: value.map(|v| serde_json::Value::String(v.to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn base_fixture(id: &str, category: &str, value: Option<&str>) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: Some(category.to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({
            "config": { "max_depth": 200 },
            "url": "https://example.com"
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![error_assertion(value)],
        source: format!("{category}/{id}.json"),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

fn group(fixture: Fixture, category: &str) -> FixtureGroup {
    FixtureGroup {
        category: category.to_string(),
        fixtures: vec![fixture],
    }
}

/// TOML with a "handle"-typed arg (`engine`), matching the crawlberg-style
/// create-engine-then-call-operation shape used by `validation_creation_failure`.
const TOML: &str = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "demo_lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "DemoLib"
result_var = "result"
returns_result = true
args = [
  { name = "engine", field = "config", type = "handle" },
  { name = "url", field = "url", type = "string" },
]

[crates.e2e.call.overrides.elixir]
module = "DemoLib"
returns_result = true
"#;

fn render(fixture: Fixture, category: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![group(fixture, category)];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files.iter().map(|f| f.content.clone()).collect::<Vec<_>>().join("\n")
}

// --- non-validation expects_error branch (plain "error_*" style fixtures) ---

#[test]
fn declared_error_value_emits_a_real_check_on_the_call_result() {
    let content = render(base_fixture("error_bad_request", "error", Some("BadRequest")), "error");

    assert!(
        content.contains("assert {:error, __reason} ="),
        "expected the error tuple to be bound for inspection, got:\n{content}"
    );
    assert!(
        content.contains("assert String.contains?(inspect(__reason), \"BadRequest\")"),
        "expected a real check against the declared error value, got:\n{content}"
    );
    // The bare, unchecked match must not remain once a value is declared.
    assert!(
        !content.contains("assert {:error, _} ="),
        "declared value must replace the untyped {{:error, _}} match, got:\n{content}"
    );
}

#[test]
fn no_declared_value_is_byte_identical_to_the_bare_match() {
    let content = render(base_fixture("error_generic", "error", None), "error");

    assert!(
        content.contains("assert {:error, _} = DemoLib.scrape(engine, \"https://example.com\")"),
        "expected the untouched bare match when no value is declared, got:\n{content}"
    );
    assert!(
        !content.contains("__reason"),
        "no declared value must not introduce the reason-binding check, got:\n{content}"
    );
}

#[test]
fn declared_value_is_escaped_for_quotes_and_backslashes() {
    let content = render(
        base_fixture("error_escaped", "error", Some("bad \"field\\name\"")),
        "error",
    );

    assert!(
        content.contains("assert String.contains?(inspect(__reason), \"bad \\\"field\\\\name\\\"\")"),
        "expected quotes and backslashes to be escaped for an Elixir string literal, got:\n{content}"
    );
}

// --- validation-category two-call shape (Task B) ---

#[test]
fn validation_category_calls_the_operation_under_test_when_creation_succeeds() {
    let content = render(
        base_fixture("validation_max_depth_too_high", "validation", Some("max_depth")),
        "validation",
    );

    // The old shape stopped after rewriting engine creation into an assertion and never
    // called `scrape/2`, so the generated test could never fail for a fixture whose error
    // is raised per-request rather than at construction. The fixed shape must call it.
    assert!(
        content.contains("DemoLib.scrape(engine, \"https://example.com\")"),
        "expected the operation under test to be called inside the {{:ok, engine}} branch, got:\n{content}"
    );
    assert!(
        content.contains("case DemoLib.create_engine("),
        "expected engine creation to be attempted via a case, not asserted unconditionally, got:\n{content}"
    );
    assert!(
        content.contains("{:ok, engine} ->"),
        "expected a success branch that continues to the operation call, got:\n{content}"
    );
    assert!(
        content.contains("{:error, __reason} ->"),
        "expected a failure branch that checks the declared value on creation failure too, got:\n{content}"
    );
}

#[test]
fn validation_category_without_declared_value_still_calls_the_operation() {
    let content = render(base_fixture("validation_no_value", "validation", None), "validation");

    assert!(
        content.contains("DemoLib.scrape(engine, \"https://example.com\")"),
        "expected the operation under test to be called even without a declared value, got:\n{content}"
    );
    assert!(
        content.contains("assert {:error, _} = DemoLib.scrape(engine, \"https://example.com\")"),
        "expected a bare error assertion on the operation call, got:\n{content}"
    );
}
