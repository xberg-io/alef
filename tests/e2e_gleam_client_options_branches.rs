use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::gleam::GleamE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn generate(has_mock: bool, has_api_key: bool) -> (String, String) {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["gleam"]
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
[crates.e2e.call.overrides.gleam]
function = "chat"
client_factory = "create_client"
client_factory_from_json = "configured_client_json"
"#,
    )
    .unwrap();
    let fixture = Fixture {
        id: "array_options".into(),
        description: "client arrays".into(),
        input: serde_json::json!({}),
        client_options: Some(BTreeMap::from([("thresholds".into(), serde_json::json!([u64::MAX]))])),
        env: has_api_key.then(|| serde_json::from_value(serde_json::json!({"api_key_var": "API_KEY"})).unwrap()),
        mock_response: has_mock.then(|| MockResponse {
            status: 200,
            body: Some(serde_json::json!({"ok": true})),
            stream_chunks: None,
            headers: BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        assertions: vec![Assertion {
            assertion_type: "not_error".into(),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let e2e = config.crates[0].e2e.clone().unwrap();
    let resolved = config.resolve().unwrap().remove(0);
    let files = GleamE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "options".into(),
                fixtures: vec![fixture],
            }],
            &e2e,
            &resolved,
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("options_test.gleam"))
        .unwrap()
        .content;
    let manifest = &files
        .iter()
        .find(|file| file.path.ends_with("gleam.toml"))
        .unwrap()
        .content;
    (source.clone(), manifest.clone())
}

#[test]
fn static_mock_json_client_appends_url_without_an_option_case_or_import() {
    let (source, manifest) = generate(true, false);
    assert!(
        source.contains("json.to_string(json.string(options_mock_url))"),
        "{source}"
    );
    assert!(!source.contains("options_base_url"), "{source}");
    assert!(!source.contains("import gleam/option"), "{source}");
    assert!(source.contains("import gleam/json"), "{source}");
    assert!(manifest.contains("gleam_json ="), "{manifest}");
    assert!(source.contains("18446744073709551615"), "{source}");
}

#[test]
fn static_no_mock_json_client_closes_object_without_a_url_branch() {
    let (source, manifest) = generate(false, false);
    assert!(!source.contains("options_base_url"), "{source}");
    assert!(!source.contains("options_mock_url"), "{source}");
    assert!(!source.contains("import gleam/option"), "{source}");
    assert!(source.contains("configured_client_json(options_json)"), "{source}");
    assert!(manifest.contains("gleam_json ="), "{manifest}");
}

#[test]
fn mock_with_live_key_fallback_keeps_both_reachable_option_variants() {
    let (source, manifest) = generate(true, true);
    assert!(source.contains("case options_base_url"), "{source}");
    assert!(source.contains("option.None ->"), "{source}");
    assert!(source.contains("option.Some(url) ->"), "{source}");
    assert!(source.contains("import gleam/option"), "{source}");
    assert!(source.contains("envoy.get(\"API_KEY\")"), "{source}");
    assert!(manifest.contains("gleam_json ="), "{manifest}");
}
