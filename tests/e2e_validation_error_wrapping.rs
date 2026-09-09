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
//! - PHP:    `try { setup_lines; call_expr; $this->fail(...); } catch (\Exception $e) { assertTrue(...); }`
//!   (or plain `$this->expectException(...); setup_lines; call_expr;` when the fixture
//!   declares no specific error value to check)
//! - C#:     `Assert.ThrowsAnyAsync<Exc>(async () => { setup_lines; await call_expr; })`
//! - Elixir: `case Module.create_engine(config) do {:error, r} -> assert ...; {:ok, engine} -> assert {:error, r} = Module.scrape(engine, url) end`
//! - Go:     `engine, createErr := pkg.CreateEngine(&cfg); assert.Error(t, createErr); return`
//! - Rust:   `let engine_result = create_engine(...); let result = match engine_result { Err(e) => Err(e), Ok(engine) => { scrape(...).await } }; assert!(result.is_err(), ...)`

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_validation_config(language: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "demo_crawler"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "DemoCrawler"
result_var = "result"
async = true
returns_result = true
options_type = "CrawlConfig"
args = [
  {{ name = "engine", field = "config", type = "handle" }},
  {{ name = "url", field = "url", type = "string" }},
]

[crates.e2e.call.overrides.ruby]
module = "DemoCrawler"

[crates.e2e.call.overrides.php]
module = "DemoCrawler"

[crates.e2e.call.overrides.csharp]
class = "DemoCrawler"

[crates.e2e.call.overrides.elixir]
module = "DemoCrawler"
returns_result = true

[crates.e2e.call.overrides.go]
alias = "demo_crawler"
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
            docs: None,
            requirements: Vec::new(),
            id: "validation_max_depth_too_high".to_string(),
            category: Some("validation".to_string()),
            description: "max_depth above allowed maximum should be rejected".to_string(),
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
            assertions: vec![Assertion {
                skip: None,
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
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }
}

fn generate_content(codegen: &dyn E2eCodegen, language: &str) -> String {
    let (e2e, resolved) = build_validation_config(language);
    let groups = vec![build_validation_fixture()];
    let files = codegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files.iter().map(|f| f.content.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn ruby_validation_setup_lines_are_inside_expect_block() {
    let content = generate_content(&RubyCodegen, "ruby");

    assert!(content.contains("expect {"), "expect block opener missing:\n{content}");
    assert!(
        content.contains("}.to raise_error"),
        "expect block closer missing:\n{content}"
    );

    let expect_pos = content.find("expect {").expect("expect { not found");
    let create_engine_pos = content.find("create_engine").expect("create_engine not found");
    assert!(
        create_engine_pos > expect_pos,
        "engine creation must appear inside expect block (after `expect {{`), \
         but found create_engine at {create_engine_pos} before expect at {expect_pos}:\n{content}"
    );
}

#[test]
fn php_validation_setup_is_caught_and_assertions_are_outside_the_handler() {
    let content = generate_content(&PhpCodegen, "php");
    let try_pos = content.find("try {").expect("try block missing");
    let create_engine_pos = content
        .find("createEngine")
        .or_else(|| content.find("create_engine"))
        .expect("engine creation call missing");
    let catch_pos = content.find("} catch (\\Exception $e) {").expect("catch missing");
    let catch_end = catch_pos
        + content[catch_pos..]
            .find("\n        }")
            .expect("catch closing brace missing");
    let instance_pos = content
        .find("$this->assertInstanceOf(")
        .expect("exception assertion missing");
    let message_pos = content.find("$this->assertTrue(").expect("message assertion missing");

    assert!(
        create_engine_pos > try_pos && create_engine_pos < catch_pos,
        "{content}"
    );
    assert!(instance_pos > catch_end && message_pos > instance_pos, "{content}");
    assert!(
        content.contains("$caughtException = $e;"),
        "the actual exception must be retained: {content}"
    );
    assert!(
        !content.contains("$this->fail("),
        "framework failures must not replace the application error: {content}"
    );
}

#[test]
fn csharp_validation_setup_lines_are_inside_throws_lambda() {
    let content = generate_content(&CSharpCodegen, "csharp");

    assert!(
        content.contains("Assert.ThrowsAnyAsync"),
        "ThrowsAny assertion missing:\n{content}"
    );

    let throws_pos = content.find("Assert.ThrowsAny").expect("ThrowsAny not found");
    let create_engine_pos = content.find("CreateEngine").or_else(|| content.find("create_engine"));
    let create_engine_pos = create_engine_pos.expect("engine creation not found in C# output");
    assert!(
        create_engine_pos > throws_pos,
        "engine creation must appear inside ThrowsAny lambda (after ThrowsAny), \
         but create_engine at {create_engine_pos} is before ThrowsAny at {throws_pos}:\n{content}"
    );
}

#[test]
fn elixir_validation_asserts_error_on_creation_or_on_the_call() {
    let content = generate_content(&ElixirCodegen, "elixir");

    assert!(
        content.contains("case DemoCrawler.create_engine("),
        "creation must be matched with `case` so both outcomes are handled:\n{content}"
    );

    assert!(
        content.contains("{:error, __reason} ->"),
        "the creation-failed branch must bind the reason to assert on it:\n{content}"
    );

    // ~keep The anti-vacuity check. The generator used to stop after asserting
    // `{:error, _}` on create_engine, so any fixture whose error is raised
    // per-request rather than at construction asserted nothing and could never
    // fail. Creation succeeding must therefore fall through to the operation and
    // assert the error there, matching the Go and Rust shapes above.
    let ok_branch = content
        .find("{:ok, engine} ->")
        .expect("creation-succeeded branch missing — the vacuous single-assert shape is back");
    let call_pos = content
        .find("DemoCrawler.scrape(engine,")
        .expect("the operation under test is never called — assertion is vacuous");
    assert!(
        call_pos > ok_branch,
        "the operation must be called inside the creation-succeeded branch:\n{content}"
    );
    assert!(
        content[ok_branch..].contains("assert {:error, __reason} ="),
        "the operation's result must be asserted to be an error:\n{content}"
    );
}

#[test]
fn go_validation_asserts_error_on_engine_creation() {
    let content = generate_content(&GoCodegen, "go");

    assert!(
        content.contains("assert.Error(t, createErr)"),
        "assert.Error(t, createErr) missing for validation fixture:\n{content}"
    );

    let assert_pos = content
        .find("assert.Error(t, createErr)")
        .expect("assert.Error not found");
    let scrape_pos = content.find("Scrape(").or_else(|| content.find("scrape("));
    if let Some(scrape_pos) = scrape_pos {
        assert!(
            assert_pos < scrape_pos,
            "assert.Error on createErr must appear before Scrape call:\n{content}"
        );
    }
}

#[test]
fn rust_validation_uses_match_to_propagate_engine_creation_error() {
    let content = generate_content(&RustE2eCodegen, "rust");

    // Must emit `let engine_result = create_engine(...)` (no .expect() on creation).
    assert!(
        content.contains("let engine_result = create_engine("),
        "engine_result binding with create_engine missing:\n{content}"
    );

    // Must NOT use `.expect("handle creation should succeed")` which would panic.
    assert!(
        !content.contains(".expect(\"handle creation should succeed\")"),
        "panicking .expect() on handle creation found — would crash before error assertion:\n{content}"
    );

    assert!(
        content.contains("match engine_result {"),
        "match engine_result block missing:\n{content}"
    );
    assert!(
        content.contains("Err(e) => Err(e),"),
        "Err propagation arm missing in match:\n{content}"
    );
    assert!(
        content.contains("Ok(engine) =>"),
        "Ok arm with unwrapped engine missing in match:\n{content}"
    );

    assert!(
        content.contains("result.is_err()"),
        "result.is_err() assertion missing:\n{content}"
    );
}
