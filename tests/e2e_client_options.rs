//! Verifies the Rust and Python e2e generators apply a fixture's `client_options` to the client
//! they build, and that a generator with no way to apply them leaves the fixture out.
//!
//! Rust writes the options into the JSON that `client_factory_from_json` reads. Python passes
//! each option as a keyword argument of `client_factory`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn build_config(language: &str, overrides: &str) -> NewAlefConfig {
    let toml_src = format!(
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
result_var = "result"
async = true
returns_result = true

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"

[crates.e2e.call.overrides.{language}]
{overrides}
"#
    );
    toml::from_str(&toml_src).expect("config parses")
}

fn build_group(client_options: Option<BTreeMap<String, serde_json::Value>>) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_client_options".to_string(),
            category: Some("smoke".to_string()),
            description: "a client with a short timeout".to_string(),
            input: serde_json::json!({ "request": { "model": "m", "messages": [] } }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::json!({ "ok": true })),
                stream_chunks: None,
                headers: BTreeMap::new(),
                delay_ms: None,
                stream_chunk_delay_ms: None,
            }),
            client_options,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Assertion::default()
            }],
            source: "smoke.json".to_string(),
            ..Fixture::default()
        }],
    }
}

fn options() -> Option<BTreeMap<String, serde_json::Value>> {
    Some(BTreeMap::from([
        ("timeout_secs".to_string(), serde_json::json!(1)),
        ("max_retries".to_string(), serde_json::json!(0)),
    ]))
}

fn generate(codegen: &dyn E2eCodegen, cfg: &NewAlefConfig, group: FixtureGroup) -> String {
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    codegen
        .generate(&[group], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds")
        .iter()
        .filter(|f| f.path.to_string_lossy().contains("smoke"))
        .map(|f| f.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const RUST_FACTORIES: &str = "crate_name = \"demo_client\"\nclient_factory = \"create_client\"\nclient_factory_from_json = \"create_client_from_json\"";

#[test]
fn rust_writes_client_options_into_the_json_the_factory_reads() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let content = generate(&RustE2eCodegen, &cfg, build_group(options()));

    assert!(
        content.contains("demo_client::create_client_from_json(&client_config.to_string())"),
        "the client must come from the JSON factory:\n{content}"
    );
    assert!(
        content.contains("\"timeout_secs\":1") && content.contains("\"max_retries\":0"),
        "every option must reach the JSON:\n{content}"
    );
    assert!(
        content.contains("client_config[\"api_key\"]") && content.contains("client_config[\"base_url\"]"),
        "the credential and the mock server address stay next to the options:\n{content}"
    );
}

#[test]
fn rust_without_client_options_keeps_the_plain_factory() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let content = generate(&RustE2eCodegen, &cfg, build_group(None));

    assert!(
        content.contains("smoke_client_options"),
        "the fixture still runs:\n{content}"
    );
    assert!(
        !content.contains("timeout_secs") && !content.contains("client_config"),
        "a fixture with no options must not build a JSON config:\n{content}"
    );
}

#[test]
fn rust_leaves_out_a_fixture_whose_call_has_no_json_factory() {
    let cfg = build_config(
        "rust",
        "crate_name = \"demo_client\"\nclient_factory = \"create_client\"",
    );
    let content = generate(&RustE2eCodegen, &cfg, build_group(options()));

    assert!(
        !content.contains("smoke_client_options"),
        "the fixture must not run with default options:\n{content}"
    );
}

#[test]
fn python_passes_client_options_as_keyword_arguments() {
    let cfg = build_config("python", "client_factory = \"create_client\"");
    let content = generate(&PythonE2eCodegen, &cfg, build_group(options()));

    assert!(
        content.contains("max_retries=0") && content.contains("timeout_secs=1"),
        "every option must be a keyword argument of the factory:\n{content}"
    );
    let factory_line = content
        .lines()
        .find(|line| line.contains("create_client("))
        .unwrap_or_else(|| panic!("no factory call:\n{content}"));
    assert!(
        factory_line.contains("api_key=\"test-key\"") && factory_line.contains("timeout_secs=1"),
        "the options sit on the same call as the credential: {factory_line}"
    );
}

#[test]
fn python_without_client_options_adds_no_keyword_arguments() {
    let cfg = build_config("python", "client_factory = \"create_client\"");
    let content = generate(&PythonE2eCodegen, &cfg, build_group(None));

    let factory_line = content
        .lines()
        .find(|line| line.contains("create_client("))
        .unwrap_or_else(|| panic!("no factory call:\n{content}"));
    assert!(!factory_line.contains("timeout_secs"), "{factory_line}");
}
