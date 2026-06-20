//! Verifies that the brew e2e codegen correctly:
//! 1. Renders mock_url_list arguments as multiple positional URL tokens.
//! 2. Uses the call's brew override function (e.g., "batch-scrape") as authoritative
//!    instead of tag-derived subcommand determination.
//!
//! This test validates batch-scrape and batch-crawl fixtures work correctly with
//! the brew CLI backend.

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

fn build_config_with_batch_calls() -> NewAlefConfig {
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

[crates.e2e.calls.batch_scrape]
function = "batch_scrape"
module = "testlib"
result_var = "result"
select_when = { category = "batch", input_has = "batch_urls" }
args = [
  { name = "urls", field = "batch_urls", type = "mock_url_list" },
]

[crates.e2e.calls.batch_scrape.overrides.brew]
function = "batch-scrape"
cli_args = ["--format", "json"]

[crates.e2e.calls.batch_crawl]
function = "batch_crawl"
module = "testlib"
result_var = "result"
select_when = { tag = "batch-crawl" }
args = [
  { name = "urls", field = "batch_urls", type = "mock_url_list" },
]

[crates.e2e.calls.batch_crawl.overrides.brew]
function = "batch-crawl"
cli_args = ["--format", "json"]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_batch_scrape_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "batch".to_string(),
        fixtures: vec![Fixture {
            id: "scrape_batch_basic".to_string(),
            category: Some("batch".to_string()),
            description: "Batch scrape multiple URLs".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "batch_urls": ["/page1", "/page2", "/page3"],
                "config": {}
            }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("total_count".to_string()),
                value: Some(serde_json::json!(3)),
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

fn build_batch_crawl_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "crawl".to_string(),
        fixtures: vec![Fixture {
            id: "crawl_batch_depth".to_string(),
            category: Some("crawl".to_string()),
            description: "Batch crawl with depth limit".to_string(),
            tags: vec!["batch-crawl".to_string()],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "batch_urls": ["/", "/path1"],
                "config": { "max_depth": 2 }
            }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("total_count".to_string()),
                value: Some(serde_json::json!(2)),
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
fn brew_renders_mock_url_list_as_positional_urls() {
    let cfg = build_config_with_batch_calls();
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_batch_scrape_fixture()];
    let files = BrewCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("Brew generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_batch.sh"))
        .expect("test_batch.sh should be emitted");

    let content = &test_file.content;

    // Verify the test function is defined.
    assert!(
        content.contains("test_scrape_batch_basic()"),
        "test function must be defined. Content:\n{content}"
    );

    // Verify all three URLs are rendered as positional arguments.
    // Each path should be preceded by the base URL variable.
    // Paths: /page1, /page2, /page3
    assert!(
        content.contains("${MOCK_SERVER_SCRAPE_BATCH_BASIC:-${MOCK_SERVER_URL}/fixtures/scrape_batch_basic}/page1"),
        "URL /page1 must be rendered with mock server base. Content:\n{content}"
    );
    assert!(
        content.contains("${MOCK_SERVER_SCRAPE_BATCH_BASIC:-${MOCK_SERVER_URL}/fixtures/scrape_batch_basic}/page2"),
        "URL /page2 must be rendered with mock server base. Content:\n{content}"
    );
    assert!(
        content.contains("${MOCK_SERVER_SCRAPE_BATCH_BASIC:-${MOCK_SERVER_URL}/fixtures/scrape_batch_basic}/page3"),
        "URL /page3 must be rendered with mock server base. Content:\n{content}"
    );
}

#[test]
fn brew_honors_override_subcommand() {
    let cfg = build_config_with_batch_calls();
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_batch_scrape_fixture()];
    let files = BrewCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("Brew generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_batch.sh"))
        .expect("test_batch.sh should be emitted");

    let content = &test_file.content;

    // Verify the subcommand is "batch-scrape" (from brew override),
    // not "scrape" (the default).
    // The command line should start with: testlib batch-scrape (not testlib scrape)
    assert!(
        content.contains("testlib batch-scrape"),
        "subcommand must be 'batch-scrape' (from override). Content:\n{content}"
    );
}

#[test]
fn brew_override_subcommand_works_with_tag_routing() {
    let cfg = build_config_with_batch_calls();
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_batch_crawl_fixture()];
    let files = BrewCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("Brew generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_crawl.sh"))
        .expect("test_crawl.sh should be emitted");

    let content = &test_file.content;

    // Verify the subcommand is "batch-crawl" (from brew override for batch_crawl call),
    // even though the fixture has a "batch-crawl" tag.
    assert!(
        content.contains("test_crawl_batch_depth()"),
        "test function must be defined. Content:\n{content}"
    );

    assert!(
        content.contains("testlib batch-crawl"),
        "subcommand must be 'batch-crawl' (from call override). Content:\n{content}"
    );

    // Verify the URLs are rendered with the correct base.
    assert!(
        content.contains("${MOCK_SERVER_CRAWL_BATCH_DEPTH:-${MOCK_SERVER_URL}/fixtures/crawl_batch_depth}/path1"),
        "URL /path1 must be rendered with mock server base. Content:\n{content}"
    );
}
