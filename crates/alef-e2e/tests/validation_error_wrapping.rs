//! Regression tests: validation-category fixtures (expects_error == true) must have
//! engine/handle creation INSIDE the error-assertion block, not before it.
//!
//! When a validation fixture passes a bad config (e.g. max_depth too high), the
//! engine creation itself raises the error. If setup_lines (engine creation) are emitted
//! before the error-assertion block, a match error / crash occurs before the assertion
//! can catch it.
//!
//! Correct shapes by language:
//! - Ruby:   `expect { setup_lines; call_expr }.to raise_error`
//! - PHP:    `$this->expectException(...); setup_lines; call_expr;`
//! - C#:     `Assert.ThrowsAnyAsync<Exc>(async () => { setup_lines; await call_expr; })`
//! - Elixir: `assert {:error, _} = Module.create_engine(config)` (no separate call)
//! - Go:     `engine, createErr := pkg.CreateEngine(&cfg); assert.Error(t, createErr); return`

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::csharp::CSharpCodegen;
use alef_e2e::codegen::elixir::ElixirCodegen;
use alef_e2e::codegen::go::GoCodegen;
use alef_e2e::codegen::php::PhpCodegen;
use alef_e2e::codegen::ruby::RubyCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

// ── helpers ──────────────────────────────────────────────────────────────────

fn build_validation_config(language: &str) -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "kreuzcrawl"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "Kreuzcrawl"
result_var = "result"
async = true
returns_result = true
args = [
  {{ name = "engine", field = "config", type = "handle" }},
  {{ name = "url", field = "url", type = "string" }},
]

[crates.e2e.call.overrides.ruby]
module = "Kreuzcrawl"

[crates.e2e.call.overrides.php]
module = "Kreuzcrawl"

[crates.e2e.call.overrides.csharp]
class = "Kreuzcrawl"

[crates.e2e.call.overrides.elixir]
module = "Kreuzcrawl"
returns_result = true

[crates.e2e.call.overrides.go]
import_alias = "kreuzcrawl"
"#,
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn build_validation_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "validation".to_string(),
        fixtures: vec![Fixture {
            id: "validation_max_depth_too_high".to_string(),
            category: Some("validation".to_string()),
            description: "max_depth above allowed maximum should be rejected".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({
                "config": { "max_depth": 200 },
                "url": "https://example.com"
            }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                field: None,
                value: Some(serde_json::Value::String("max_depth".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "validation.json".to_string(),
            http: None,
        }],
    }
}

fn generate_content(codegen: &dyn E2eCodegen, language: &str) -> String {
    let (e2e, resolved) = build_validation_config(language);
    let groups = vec![build_validation_fixture()];
    let files = codegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files.iter().map(|f| f.content.clone()).collect::<Vec<_>>().join("\n")
}

// ── Ruby ─────────────────────────────────────────────────────────────────────

#[test]
fn ruby_validation_setup_lines_are_inside_expect_block() {
    let content = generate_content(&RubyCodegen, "ruby");

    // The expect { } block must contain setup lines (engine creation) before call_expr.
    assert!(content.contains("expect {"), "expect block opener missing:\n{content}");
    assert!(
        content.contains("}.to raise_error"),
        "expect block closer missing:\n{content}"
    );

    // Engine creation (create_engine) must appear INSIDE the expect block, not before it.
    // We verify by checking that `create_engine` appears after `expect {` in the output.
    let expect_pos = content.find("expect {").expect("expect { not found");
    let create_engine_pos = content.find("create_engine").expect("create_engine not found");
    assert!(
        create_engine_pos > expect_pos,
        "engine creation must appear inside expect block (after `expect {{`), \
         but found create_engine at {create_engine_pos} before expect at {expect_pos}:\n{content}"
    );
}

// ── PHP ───────────────────────────────────────────────────────────────────────

#[test]
fn php_validation_setup_lines_are_after_expect_exception() {
    let content = generate_content(&PhpCodegen, "php");

    assert!(
        content.contains("$this->expectException"),
        "expectException call missing:\n{content}"
    );

    // Engine creation (createEngine) must appear AFTER expectException.
    // PHP uses camelCase constructors: createEngine, createHandle, etc.
    let expect_pos = content
        .find("$this->expectException")
        .expect("expectException not found");
    let create_engine_pos = content
        .find("createEngine")
        .or_else(|| content.find("create_engine"))
        .expect("engine creation call (createEngine or create_engine) not found");
    assert!(
        create_engine_pos > expect_pos,
        "engine creation must appear after expectException, \
         but engine creation at {create_engine_pos} is before expectException at {expect_pos}:\n{content}"
    );
}

// ── C# ────────────────────────────────────────────────────────────────────────

#[test]
fn csharp_validation_setup_lines_are_inside_throws_lambda() {
    let content = generate_content(&CSharpCodegen, "csharp");

    // Assert.ThrowsAnyAsync must be present (async validation fixture).
    assert!(
        content.contains("Assert.ThrowsAnyAsync") || content.contains("Assert.ThrowsAny"),
        "ThrowsAny assertion missing:\n{content}"
    );

    // The lambda body must contain the engine creation.
    // Verify create_engine appears after the lambda opener `=> {` or `async () =>`.
    let throws_pos = content.find("Assert.ThrowsAny").expect("ThrowsAny not found");
    let create_engine_pos = content.find("CreateEngine").or_else(|| content.find("create_engine"));
    let create_engine_pos = create_engine_pos.expect("engine creation not found in C# output");
    assert!(
        create_engine_pos > throws_pos,
        "engine creation must appear inside ThrowsAny lambda (after ThrowsAny), \
         but create_engine at {create_engine_pos} is before ThrowsAny at {throws_pos}:\n{content}"
    );
}

// ── Elixir ────────────────────────────────────────────────────────────────────

#[test]
fn elixir_validation_emits_error_assertion_on_engine_creation() {
    let content = generate_content(&ElixirCodegen, "elixir");

    // Must emit `assert {:error, _} = Module.create_engine(...)`.
    assert!(
        content.contains("assert {:error, _} ="),
        "error assertion pattern missing:\n{content}"
    );

    // The assertion must be on the engine creation call, not on a separate scrape call.
    assert!(
        content.contains("assert {:error, _} =") && content.contains("create_engine"),
        "assert {{:error, _}} must wrap create_engine:\n{content}"
    );

    // Must NOT have `{:ok, engine} = ...` which would crash on bad config.
    assert!(
        !content.contains("{:ok, engine}"),
        "{{:ok, engine}} = ... pattern found — would crash on validation fixture:\n{content}"
    );
}

// ── Go ────────────────────────────────────────────────────────────────────────

#[test]
fn go_validation_asserts_error_on_engine_creation() {
    let content = generate_content(&GoCodegen, "go");

    // Must emit `assert.Error(t, createErr)` for the engine creation error.
    assert!(
        content.contains("assert.Error(t, createErr)"),
        "assert.Error(t, createErr) missing for validation fixture:\n{content}"
    );

    // Must NOT use `return` alone (old behavior) as the sole error handling.
    // The new behavior is `assert.Error(t, createErr)\n\t\treturn`.
    let assert_pos = content
        .find("assert.Error(t, createErr)")
        .expect("assert.Error not found");
    // Verify it appears in engine creation context (before the main call).
    let scrape_pos = content.find("Scrape(").or_else(|| content.find("scrape("));
    if let Some(scrape_pos) = scrape_pos {
        assert!(
            assert_pos < scrape_pos,
            "assert.Error on createErr must appear before Scrape call:\n{content}"
        );
    }
}
