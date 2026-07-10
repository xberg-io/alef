//! Regression: Zig 0.16's `b.addTest(.{...})` hashes the output binary path off the
//! artifact `.name`. Without an explicit name, every `addTest` call defaults to
//! `"test"`, colliding in the cache — only one binary survives, and every other
//! `addRunArtifact` invocation fails with:
//!
//! ```text
//! error: failed to spawn and capture stdio from ./../e2e/zig/.zig-cache/o/<hash>/test:
//!        FileNotFound
//! ```
//!
//! Each `b.addTest(.{...})` block in the generated `build.zig` must include a
//! unique `.name = "..."` field so per-test binaries get distinct cache hashes.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo_crawler"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "demo_crawler"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "url", field = "url", type = "string" },
]
"#;

fn fixture_for(category: &str, id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some(category.to_string()),
        description: format!("{category} fixture {id}"),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "url": "https://example.com" }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::String("<html></html>".to_string())),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
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
        source: format!("{category}.json"),
        http: None,
    }
}

fn render_build_zig(groups: Vec<FixtureGroup>) -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.zig"))
        .expect("build.zig generated")
        .content
        .clone()
}

#[test]
fn add_test_calls_set_unique_name_per_test_module() {
    let groups = vec![
        FixtureGroup {
            category: "encoding".to_string(),
            fixtures: vec![fixture_for("encoding", "double_encoded")],
        },
        FixtureGroup {
            category: "crawl".to_string(),
            fixtures: vec![fixture_for("crawl", "basic")],
        },
    ];
    let content = render_build_zig(groups);

    let add_test_count = content.matches("b.addTest(.{").count();
    let named_add_test_count = content.matches(".name = \"").count();
    assert!(
        add_test_count >= 2,
        "expected at least two addTest blocks for two fixture groups:\n{content}"
    );
    assert!(
        named_add_test_count >= add_test_count,
        "each addTest block must include a .name field; \
         found {named_add_test_count} .name entries for {add_test_count} addTest blocks:\n{content}"
    );

    assert!(
        content.contains(".name = \"encoding_test\","),
        "encoding test module must set .name = \"encoding_test\":\n{content}"
    );
    assert!(
        content.contains(".name = \"crawl_test\","),
        "crawl test module must set .name = \"crawl_test\":\n{content}"
    );
}
