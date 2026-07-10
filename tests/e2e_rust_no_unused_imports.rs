//! Regression: Rust e2e codegen must not emit optional type imports (or other
//! optional `use` statements) when the rendered test body never references the imported
//! symbol. Under `-D unused_imports`, an unused import fails the build.
//!
//! The typical case is a handle-arg call where every fixture passes `input.config` as
//! null/empty — `render_rust_arg` emits `create_engine(None)` with no config type
//! reference in the body, but the file-level import would still be emitted.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config(toml: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture_without_config(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("encoding".to_string()),
        description: "scrape with default engine (no config)".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "url": "https://example.com" }),
        mock_response: None,
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
        source: "encoding.json".to_string(),
        http: None,
    }
}

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "demo_engine"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "demo_engine"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "engine", field = "config", type = "handle" },
  { name = "url", field = "url", type = "string" },
]
"#;

fn render(group: FixtureGroup) -> String {
    let (e2e, resolved) = build_config(CONFIG_TOML);
    let files = RustE2eCodegen
        .generate(&[group], &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .filter(|f| {
            f.path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| name.ends_with("_test.rs"))
        })
        .map(|f| f.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn omits_crawl_config_import_when_body_has_no_reference() {
    let group = FixtureGroup {
        category: "encoding".to_string(),
        fixtures: vec![fixture_without_config("encoding_double_encoded")],
    };
    let content = render(group);
    assert!(
        !content.contains("use demo_engine::EngineConfig"),
        "config import emitted for a body that never references it (would trip -D unused_imports):\n{content}"
    );
    assert!(
        content.contains("use demo_engine::create_engine"),
        "create_engine import missing from a body that uses it:\n{content}"
    );
}

#[test]
fn keeps_config_body_when_fixture_provides_it() {
    let mut fixture = fixture_without_config("encoding_with_config");
    fixture.input = serde_json::json!({
        "url": "https://example.com",
        "config": { "max_depth": 5 }
    });
    let group = FixtureGroup {
        category: "encoding".to_string(),
        fixtures: vec![fixture],
    };
    let content = render(group);
    assert!(
        content.contains("let engine_config = serde_json::from_str"),
        "config deserialization missing for a fixture that provides config:\n{content}"
    );
    assert!(
        content.contains("use demo_engine::create_engine"),
        "create_engine import missing from a body that uses it:\n{content}"
    );
}
