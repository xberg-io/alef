use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::all_generators;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn configuration() -> NewAlefConfig {
    toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "demo_client"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "ping"
module = "demo_client"
result_is_simple = true
returns_result = true
[crates.e2e.call.overrides.node]
client_factory = "createClient"
client_factory_from_json = "createClientFromJson"
[crates.e2e.calls.free]
function = "create_client"
result_is_simple = true
returns_result = true
[crates.e2e.calls.free.overrides.node]
function = "createClient"
client_factory = ""
"#,
    )
    .expect("configuration")
}

fn fixture(id: &str, call: Option<&str>) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("imports".into()),
        description: id.to_string(),
        call: call.map(str::to_string),
        input: serde_json::json!({}),
        assertions: vec![Assertion {
            assertion_type: "not_error".into(),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn generate(mixed: bool) -> String {
    let config = configuration();
    let e2e = config.crates[0].e2e.as_ref().expect("e2e");
    let resolved = config.clone().resolve().expect("resolved").remove(0);
    let mut fixtures = vec![fixture("free_collision", Some("free"))];
    if mixed {
        let mut configured = fixture("configured_client", None);
        configured.client_options = Some(BTreeMap::from([(
            "blocked_networks".into(),
            serde_json::json!(["10.0.0.0/8"]),
        )]));
        configured.mock_response = Some(MockResponse {
            status: 200,
            body: Some(serde_json::json!({})),
            stream_chunks: None,
            headers: BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        });
        fixtures.push(configured);
    }
    let generators = all_generators();
    let node = generators
        .iter()
        .find(|generator| generator.language_name() == "node")
        .expect("node generator");
    let files = node
        .generate(
            &[FixtureGroup {
                category: "imports".into(),
                fixtures,
            }],
            e2e,
            &resolved,
            &[],
            &[],
            &[],
            &[],
        )
        .expect("generation");
    files
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("tests/imports.test.ts"))
        .expect("test file")
        .content
}

fn assert_free_factory_export(output: &str) {
    let import = output
        .lines()
        .find(|line| line.starts_with("import {") && line.contains("from \"demo_client\""))
        .expect("binding import");
    let names: Vec<_> = import
        .split([',', '{', '}', ' '])
        .filter(|name| !name.is_empty())
        .collect();
    assert!(
        names.contains(&"createClient"),
        "free callable export must remain imported: {import}"
    );
    assert!(
        output.contains("const result = createClient();"),
        "named free call must use the resolved export:\n{output}"
    );
    assert!(
        !output.contains("client.createClient("),
        "free call must not become a client method:\n{output}"
    );
}

#[test]
fn free_function_named_like_default_client_factory_keeps_its_import() {
    let output = generate(false);
    assert_free_factory_export(&output);
    assert_eq!(
        output.matches("const client =").count(),
        0,
        "free fixture must not create a client:\n{output}"
    );
}

#[test]
fn free_factory_export_remains_imported_next_to_json_client_fixture() {
    let output = generate(true);
    assert_free_factory_export(&output);
    assert!(
        output.contains("createClientFromJson"),
        "JSON factory must be imported and called:\n{output}"
    );
    assert_eq!(
        output.matches("const client = createClientFromJson(").count(),
        1,
        "only the configured fixture creates a client:\n{output}"
    );
}
