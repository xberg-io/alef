//! Verifies the Gleam e2e codegen emits client-object instantiation when
//! `CallOverride.client_factory` is set, and falls back to flat function calls
//! when it is absent.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::gleam::GleamE2eCodegen;
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

fn render_gleam_smoke(toml: &str, fixture_id: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    let files = GleamE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.gleam"))
        .expect("smoke_test.gleam is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

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
///   2. call the method with client as first argument
///   3. NOT call the module-level function directly without client
#[test]
fn with_client_factory_emits_client_instantiation() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.gleam]
client_factory = "create_client"
"#,
        BASE_TOML
    );
    let rendered = render_gleam_smoke(&toml, "smoke_basic");

    assert!(
        rendered.contains("create_client("),
        "must call create_client factory. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("client,"),
        "must pass client as first argument. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("create_client(\"test-key\""),
        "must call create_client with api key. Rendered:\n{rendered}"
    );
}

/// When `client_factory` is absent, the generator must fall back to the flat
/// module-function call pattern. This ensures no regression.
#[test]
fn without_client_factory_emits_flat_function_call() {
    let rendered = render_gleam_smoke(BASE_TOML, "smoke_basic");

    assert!(
        rendered.contains("liter_llm.chat("),
        "must call module-level function directly. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("create_client("),
        "must NOT call create_client when client_factory is absent. Rendered:\n{rendered}"
    );
}

/// Verify the Erlang startup shim gracefully falls back when Elixir is absent.
#[test]
fn erlang_startup_shim_has_graceful_fallback() {
    let cfg: NewAlefConfig = toml::from_str(BASE_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group("smoke_basic")];
    let files = GleamE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    let startup_erl = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("e2e_startup.erl"))
        .expect("e2e_startup.erl is emitted")
        .content
        .clone();

    assert!(
        startup_erl.contains("case application:ensure_all_started(elixir)"),
        "must use case expression for elixir startup. Rendered:\n{startup_erl}"
    );
    assert!(
        startup_erl.contains("{error, _} -> ok"),
        "must have graceful error fallback. Rendered:\n{startup_erl}"
    );
}
