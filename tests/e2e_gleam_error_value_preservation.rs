//! Verifies the Gleam e2e codegen preserves the declared `error` assertion value
//! (`Assertion.value`) instead of dropping it in favor of a bare `|> should.be_error()`,
//! for both the flat-call and client-factory call shapes.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::gleam::GleamE2eCodegen;
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

fn fixture(id: &str, value: Option<&str>) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "text": "hello" }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![error_assertion(value)],
        source: "smoke.json".to_string(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

fn group(f: Fixture) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![f],
    }
}

const FLAT_TOML: &str = r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "demo-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "demo_lib"
result_var = "result"

[[crates.e2e.call.args]]
name = "text"
field = "input.text"
type = "string"
"#;

const CLIENT_FACTORY_TOML: &str = r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "demo-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "demo_lib"
result_var = "result"

[[crates.e2e.call.args]]
name = "text"
field = "input.text"
type = "string"

[crates.e2e.call.overrides.gleam]
client_factory = "create_client"
"#;

fn render(toml: &str, f: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![group(f)];
    let files = GleamE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().contains("smoke_test.gleam"))
        .expect("smoke_test.gleam is emitted")
        .content
        .clone()
}

#[test]
fn declared_error_value_emits_a_real_check_on_the_flat_call() {
    let content = render(FLAT_TOML, fixture("error_bad_request", Some("BadRequest")));

    assert!(
        content.contains("let assert Error(__reason) = __result"),
        "expected the error to be bound for inspection, got:\n{content}"
    );
    assert!(
        content.contains("should.be_true(string.contains(string.inspect(__reason), \"BadRequest\"))"),
        "expected a real check against the declared error value, got:\n{content}"
    );
    assert!(
        !content.contains("|> should.be_error()"),
        "declared value must replace the untyped should.be_error() check, got:\n{content}"
    );
}

#[test]
fn no_declared_value_is_byte_identical_to_the_bare_check() {
    let content = render(FLAT_TOML, fixture("error_generic", None));

    assert!(
        content.contains("demo_lib.process(\"hello\") |> should.be_error()"),
        "expected the untouched bare check when no value is declared, got:\n{content}"
    );
    assert!(
        !content.contains("__reason"),
        "no declared value must not introduce the reason-binding check, got:\n{content}"
    );
}

#[test]
fn declared_value_is_escaped_for_quotes_and_backslashes() {
    let content = render(FLAT_TOML, fixture("error_escaped", Some("bad \"field\\name\"")));

    assert!(
        content.contains("string.contains(string.inspect(__reason), \"bad \\\"field\\\\name\\\"\")"),
        "expected quotes and backslashes to be escaped for a Gleam string literal, got:\n{content}"
    );
}

#[test]
fn declared_error_value_emits_a_real_check_with_client_factory() {
    let content = render(
        CLIENT_FACTORY_TOML,
        fixture("error_bad_request_client", Some("BadRequest")),
    );

    assert!(
        content.contains("create_client("),
        "expected the client factory to still be called, got:\n{content}"
    );
    assert!(
        content.contains("let assert Error(__reason) = __result"),
        "expected the error to be bound for inspection, got:\n{content}"
    );
    assert!(
        content.contains("should.be_true(string.contains(string.inspect(__reason), \"BadRequest\"))"),
        "expected a real check against the declared error value, got:\n{content}"
    );
}

#[test]
fn declared_error_value_pulls_in_the_string_import() {
    let content = render(FLAT_TOML, fixture("error_bad_request", Some("BadRequest")));

    assert!(
        content.contains("import gleam/string"),
        "expected gleam/string to be imported for string.contains/inspect, got:\n{content}"
    );
}
