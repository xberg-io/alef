//! Verifies the Elixir e2e codegen respects a keyword-opts threshold:
//! - 2+ trailing optional params → use keyword form for all optional args
//! - 1 or 0 trailing optional params → use positional form for json_object args
//!
//! This prevents syntax errors like `func(path, mime_type: "...", "{}")` where
//! a positional arg comes after keyword args. Aligns with the Rustler backend's
//! threshold logic.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config_with_args(args_toml: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
module = "MyLib"
result_var = "result"
returns_result = true
args = [
  {args_toml}
]
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture_with_input(input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            id: "test_fixture".to_string(),
            category: Some("test".to_string()),
            description: "test fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input,
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
            source: "test/test_fixture.json".to_string(),
            http: None,
        }],
    }
}

/// Single optional json_object config arg → emit as positional (no keyword form)
/// This aligns with positional-default facades like extract_sync(path, config \\ nil).
#[test]
fn single_optional_config_emits_positional() {
    let args_toml = r#"
  { name = "path", field = "input.path", type = "file_path" },
  { name = "config", field = "input.config", type = "json_object", optional = true }
"#;
    let (e2e, resolved) = build_config_with_args(args_toml);
    let input = serde_json::json!({
        "path": "test.txt",
        "config": { "option1": true }
    });
    let groups = vec![fixture_with_input(input)];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_test.exs"))
        .expect("Elixir test file is emitted");

    let body = &test_file.content;

    let extract_call = body
        .lines()
        .find(|line| line.contains("MyLib.extract"))
        .expect("extract call found");

    assert!(
        extract_call.contains("extract(\"../../test_documents/test.txt\", \"{") && extract_call.contains("}\")"),
        "single optional config should emit as positional JSON string, got:\n{extract_call}"
    );
}

/// Two optional params (mime_type, config) → emit both as keyword form
/// This matches keyword-opts facades like extract_file_async(path, mime_type: "...", config: "...").
#[test]
fn two_optional_params_emit_keyword() {
    let args_toml = r#"
  { name = "path", field = "input.path", type = "file_path" },
  { name = "mime_type", field = "input.mime_type", type = "string", optional = true },
  { name = "config", field = "input.config", type = "json_object", optional = true }
"#;
    let (e2e, resolved) = build_config_with_args(args_toml);
    let input = serde_json::json!({
        "path": "test.txt",
        "mime_type": "text/plain",
        "config": { "option1": true }
    });
    let groups = vec![fixture_with_input(input)];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_test.exs"))
        .expect("Elixir test file is emitted");

    let body = &test_file.content;

    assert!(
        body.contains("mime_type:") && body.contains("config:"),
        "two optional params should emit as keywords, got:\n{body}"
    );

    let mime_pos = body.find("mime_type:").expect("mime_type keyword found");
    let config_pos = body.find("config:").expect("config keyword found");
    assert!(
        mime_pos < config_pos,
        "mime_type should come before config in the call, got:\n{body}"
    );
}
