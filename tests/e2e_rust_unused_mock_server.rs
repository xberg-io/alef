//! Regression: Rust e2e codegen must not emit `let mock_server = ...` for fixtures
//! that never reference `mock_server.url` in their bodies. Under `-D warnings` (the
//! demo_crawler CI policy), an unused variable triggers `unused_variables` and fails
//! the build.
//!
//! Error-path fixtures are the typical case: the mock server is needed to hold the
//! HTTP listener alive while the call fails, but the call expression itself does
//! not reference `mock_server.url` (the URL is passed via a different field, or the
//! error fires before any network I/O).
//!
//! Correct shape: `let _mock_server = MockServer::start(...).await;` — the leading
//! underscore silences `unused_variables` without dropping the server early
//! (the binding still owns the JoinHandle, so Drop only runs at scope exit).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn build_config(toml: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn error_fixture_with_mock() -> FixtureGroup {
    FixtureGroup {
        category: "error".to_string(),
        fixtures: vec![Fixture {
            id: "request_returns_500".to_string(),
            category: Some("error".to_string()),
            description: "server 500 should surface as Err".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "url": "https://example.com" }),
            mock_response: Some(MockResponse {
                status: 500,
                body: Some(serde_json::json!({"error": "boom"})),
                stream_chunks: None,
                headers: std::collections::BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                field: None,
                value: Some(serde_json::Value::String("500".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "error.json".to_string(),
            http: None,
        }],
    }
}

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["rust"]

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
async = true
returns_result = true
args = [
  { name = "url", field = "url", type = "string" },
]
"#;

fn render(group: FixtureGroup) -> String {
    let (e2e, resolved) = build_config(CONFIG_TOML);
    let files = RustE2eCodegen
        .generate(&[group], &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files.iter().map(|f| f.content.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn error_fixture_without_url_reference_emits_underscored_mock_server() {
    let content = render(error_fixture_with_mock());
    assert!(
        content.contains("let _mock_server = MockServer::start("),
        "error fixture that never reads mock_server.url must bind to `_mock_server` \
         to satisfy `-D unused_variables`:\n{content}"
    );
    assert!(
        !content.contains("let mock_server = MockServer::start("),
        "unprefixed `let mock_server = ...` would trip unused_variables in error fixtures:\n{content}"
    );
}
