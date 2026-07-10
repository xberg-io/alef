//! Verifies that the brew e2e codegen correctly emits skip comments (not failing
//! tests) for fixtures routed to calls marked `unsupported_in = { brew = "..." }`.
//!
//! This test validates the generic per-backend unsupported-call mechanism that
//! prevents structural limitation (e.g., complex enum serialization) from causing
//! silent test omissions.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::brew::BrewCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn resolve_one(
    cfg: &NewAlefConfig,
) -> (
    alef::core::config::ResolvedCrateConfig,
    alef::core::config::e2e::E2eConfig,
) {
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

fn build_config_with_unsupported_call() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["brew"]

[crates.e2e.call]
function = "scrape"
module = "testlib"
result_var = "result"
args = [
  { name = "url", field = "url", type = "mock_url" },
]

[crates.e2e.calls.interact]
function = "interact"
module = "testlib"
result_var = "result"
select_when = { input_has = "actions" }
unsupported_in = { brew = "interact requires serializing Vec<PageAction> to JSON CLI arguments" }
args = [
  { name = "url", field = "url", type = "mock_url" },
  { name = "actions", field = "actions", type = "json_object" },
]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_interact_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "interaction".to_string(),
        fixtures: vec![Fixture {
            id: "interact_click".to_string(),
            category: Some("interaction".to_string()),
            description: "click an element".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "url": "http://example.com",
                "actions": [{"type": "click", "selector": "#button"}]
            }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("final_url".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn brew_emits_skip_comment_for_unsupported_in_call() {
    let cfg = build_config_with_unsupported_call();
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_interact_fixture()];
    let files = BrewCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("Brew generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_interaction.sh"))
        .expect("test_interaction.sh should be emitted");

    let content = &test_file.content;

    assert!(
        content.contains("# SKIP [brew unsupported]"),
        "skip comment with [brew unsupported] marker must appear. Content:\n{content}"
    );

    assert!(
        content.contains("interact requires serializing Vec<PageAction>"),
        "skip comment must include the documented reason. Content:\n{content}"
    );

    assert!(
        content.contains("return 0"),
        "skip comment must be followed by 'return 0' so the test passes. Content:\n{content}"
    );

    assert!(
        content.contains("test_interact_click()"),
        "test function must still be defined (not silently omitted). Content:\n{content}"
    );
}
