use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, all_generators};
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn configuration(language: &str) -> NewAlefConfig {
    toml::from_str(&format!(
        r#"
[workspace]
languages = ["{language}"]
[[crates]]
name = "demo_client"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "chat"
module = "demo_client"
result_is_simple = true
[crates.e2e.call.overrides.{language}]
function = "chat"
client_factory = "create_client"
php_client_factory = "createClient"
client_factory_from_json = "configured_client_json"
"#
    ))
    .expect("configuration parses")
}

fn group() -> FixtureGroup {
    FixtureGroup {
        category: "options".into(),
        fixtures: vec![Fixture {
            id: "array_options".into(),
            description: "array options reach the client".into(),
            input: serde_json::json!({}),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::json!({"ok": true})),
                stream_chunks: None,
                headers: BTreeMap::new(),
                delay_ms: None,
                stream_chunk_delay_ms: None,
            }),
            client_options: Some(BTreeMap::from([
                (
                    "blocked_networks".into(),
                    serde_json::json!(["127.0.0.0/8", "quote\"\\\n"]),
                ),
                ("thresholds".into(), serde_json::json!([1, 2.5, -3])),
                ("empty".into(), serde_json::json!([])),
            ])),
            assertions: vec![Assertion {
                assertion_type: "not_error".into(),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }],
    }
}

#[test]
fn group_b_json_factories_receive_array_options_and_runtime_routing() {
    for language in ["swift", "ruby", "elixir", "php", "gleam", "zig", "r"] {
        let cfg = configuration(language);
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let generators = all_generators();
        let generator = generators.iter().find(|g| g.language_name() == language).unwrap();
        let files = generator
            .generate(&[group()], &e2e, &resolved, &[], &[], &[], &[])
            .unwrap();
        let source = files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n");
        assert!(
            source.contains("configured_client_json"),
            "{language}: JSON factory missing:\n{source}"
        );
        for option in ["blocked_networks", "thresholds", "empty"] {
            assert!(source.contains(option), "{language}: {option} absent:\n{source}");
        }
        assert!(
            source.contains("MOCK_SERVER_URL") || source.contains("AlefE2EMockServer"),
            "{language}: mock routing missing"
        );
    }
}
