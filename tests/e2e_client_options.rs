//! Verifies scalar and list options reach Rust/Python clients, and unconfigured generators
//! leave fixtures out rather than executing them with default options.
//!
//! Rust writes the options into the JSON that `client_factory_from_json` reads. Python passes
//! each option as a keyword argument of `client_factory`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::codegen::{E2eCodegen, InclusionDecision, all_generators, fixture_inclusion};
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
    build_group_with_mock(client_options, true)
}

fn build_group_with_mock(client_options: Option<BTreeMap<String, serde_json::Value>>, with_mock: bool) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_client_options".to_string(),
            category: Some("smoke".to_string()),
            description: "a client with a short timeout".to_string(),
            input: serde_json::json!({ "request": { "model": "m", "messages": [] } }),
            mock_response: with_mock.then(|| MockResponse {
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
fn rust_embedded_mock_preserves_response_delay_for_plain_and_streaming_routes() {
    let cfg = build_config("rust", RUST_FACTORIES);
    for streaming in [false, true] {
        let mut group = build_group(None);
        let mock = group.fixtures[0].mock_response.as_mut().unwrap();
        mock.delay_ms = Some(137);
        mock.stream_chunks = streaming.then(|| vec![serde_json::json!({"text": "hello"})]);
        let output = generate(&RustE2eCodegen, &cfg, group);
        assert!(
            output.contains("delay_ms: Some(137)"),
            "streaming={streaming}: {output}"
        );
    }
}

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

fn keyword_options() -> Option<BTreeMap<String, serde_json::Value>> {
    Some(BTreeMap::from([("class".to_string(), serde_json::json!(1))]))
}

#[test]
fn python_leaves_out_a_fixture_whose_client_option_is_a_python_keyword() {
    let cfg = build_config("python", "client_factory = \"create_client\"");
    let content = generate(&PythonE2eCodegen, &cfg, build_group(keyword_options()));

    assert!(
        !content.contains("class=1") && !content.contains("smoke_client_options"),
        "a keyword argument named `class` is a syntax error, so the fixture must not be emitted:\n{content}"
    );
}

#[test]
fn python_applies_client_options_when_the_fixture_has_no_mock_response() {
    let cfg = build_config("python", "client_factory = \"create_client\"");
    let content = generate(&PythonE2eCodegen, &cfg, build_group_with_mock(options(), false));

    let factory_line = content
        .lines()
        .find(|line| line.contains("create_client("))
        .unwrap_or_else(|| panic!("no factory call:\n{content}"));
    assert!(
        factory_line.contains("timeout_secs=1") && factory_line.contains("max_retries=0"),
        "a client with no mock server still gets the options: {factory_line}"
    );
}

#[test]
fn rust_applies_client_options_when_the_fixture_has_no_mock_response() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let content = generate(&RustE2eCodegen, &cfg, build_group_with_mock(options(), false));

    assert!(
        content.contains("\"timeout_secs\":1") && content.contains("\"max_retries\":0"),
        "the Rust client gets the options with no mock server too:\n{content}"
    );
    assert!(
        !content.contains("client_config[\"base_url\"]"),
        "no mock server, so no mock address:\n{content}"
    );
}

/// Whether a generated file runs the fixture: a line spells the fixture id, in any casing or with
/// any separator, and the line is neither a comment nor a skipped placeholder case. Zig records an
/// excluded fixture as a `// skipped:` comment and WASM as an `it.skip(...)` case; both leave the
/// fixture out.
fn mentions_fixture(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || line.contains(".skip(") {
            return false;
        }
        let squashed: String = line
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        squashed.contains("smokeclientoptions")
    })
}

#[test]
fn every_unconfigured_language_leaves_out_a_fixture_with_client_options() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let with_options = build_group(options()).fixtures.remove(0);
    let without_options = build_group(None).fixtures.remove(0);
    let mut checked = 0;
    for generator in all_generators() {
        let language = generator.language_name();
        if language == "rust" || language == "python" {
            continue;
        }
        assert_eq!(
            fixture_inclusion(&without_options, language, &e2e),
            InclusionDecision::Include,
            "{language}: the fixture without options is the control"
        );
        match fixture_inclusion(&with_options, language, &e2e) {
            InclusionDecision::Exclude(reason) => {
                assert!(reason.contains("client options"), "{language}: {reason}");
            }
            InclusionDecision::Include => panic!("{language} must leave out a fixture with client_options"),
        }
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} languages were checked");
}

#[test]
fn no_unconfigured_language_emits_a_fixture_with_client_options() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let mut controls_that_emit = 0;
    let mut leaked: Vec<String> = Vec::new();
    for generator in all_generators() {
        let language = generator.language_name();
        if language == "rust" || language == "python" {
            continue;
        }
        // A generator that returns an error or panics (the C generator panics on an argument whose
        // type the config does not name) cannot render the fixture.
        let emit = |group: FixtureGroup| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generator
                    .generate(&[group], &e2e, &resolved, &[], &[], &[], &[])
                    .ok()
                    .map(|files| files.iter().any(|file| mentions_fixture(&file.content)))
            }))
            .unwrap_or(None)
        };
        // A language whose generator cannot render the control fixture at all says nothing here.
        if emit(build_group(None)) != Some(true) {
            continue;
        }
        controls_that_emit += 1;
        if emit(build_group(options())) != Some(false) {
            leaked.push(language.to_string());
        }
    }
    assert!(
        controls_that_emit >= 5,
        "only {controls_that_emit} languages emit the control fixture, so the check proves little"
    );
    assert!(
        leaked.is_empty(),
        "these languages emitted a fixture with client_options: {leaked:?}"
    );
}

#[test]
fn python_and_rust_preserve_array_option_values() {
    let values = Some(BTreeMap::from([
        (
            "blocked_networks".into(),
            serde_json::json!(["127.0.0.0/8", "quote\"slash\\"]),
        ),
        ("thresholds".into(), serde_json::json!([-1, 0, 1.5])),
        ("empty".into(), serde_json::json!([])),
    ]));
    let python = generate(
        &PythonE2eCodegen,
        &build_config("python", "client_factory = \"create_client\""),
        build_group(values.clone()),
    );
    assert!(python.contains("blocked_networks=["), "{python}");
    assert!(python.contains("thresholds=[-1, 0, 1.5]"), "{python}");
    assert!(python.contains("empty=[]"), "{python}");
    let rust = generate(
        &RustE2eCodegen,
        &build_config("rust", RUST_FACTORIES),
        build_group(values),
    );
    assert!(rust.contains("\"blocked_networks\":["), "{rust}");
    assert!(rust.contains("\"thresholds\":[-1,0,1.5]"), "{rust}");
    assert!(rust.contains("\"empty\":[]"), "{rust}");
}

#[test]
fn python_prefers_the_configured_json_factory_for_option_lists() {
    let cfg = build_config(
        "python",
        "client_factory = \"create_client\"\nclient_factory_from_json = \"create_client_from_json\"",
    );
    let content = generate(
        &PythonE2eCodegen,
        &cfg,
        build_group(Some(BTreeMap::from([
            ("blocked_networks".into(), serde_json::json!(["127.0.0.0/8"])),
            ("class".into(), serde_json::json!(1)),
        ]))),
    );
    assert!(
        content.contains("from demo_client import create_client_from_json"),
        "{content}"
    );
    assert!(content.contains("create_client_from_json(json.dumps("), "{content}");
    assert!(content.contains("blocked_networks"), "{content}");
    assert!(content.contains("smoke_client_options"), "{content}");
    assert!(!content.contains("class=1"), "{content}");
}

#[test]
fn every_registered_client_generator_admits_a_configured_json_factory() {
    let cfg = build_config("rust", RUST_FACTORIES);
    let fixture = build_group(options()).fixtures.remove(0);
    let mut count = 0;
    for generator in all_generators() {
        let language = generator.language_name();
        let mut e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        e2e.call.overrides.insert(
            language.into(),
            alef::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                client_factory_from_json: Some("create_client_from_json".into()),
                ..Default::default()
            },
        );
        if matches!(language, "brew" | "homebrew" | "php_ext") {
            assert!(
                matches!(
                    fixture_inclusion(&fixture, language, &e2e),
                    InclusionDecision::Exclude(_)
                ),
                "{language}: packaging smoke does not execute client fixtures"
            );
            continue;
        }
        assert_eq!(
            fixture_inclusion(&fixture, language, &e2e),
            InclusionDecision::Include,
            "{language}"
        );
        count += 1;
    }
    assert_eq!(count, 18, "every registered client fixture generator was checked");
}
