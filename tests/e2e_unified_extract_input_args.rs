use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::codegen::r::RCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["dart", "swift", "r"]

[[crates]]
name = "xberg"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "xberg"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
module = "xberg"
result_var = "result"
async = true
args = [
  { name = "input", field = "input", type = "json_object", owned = true, element_type = "ExtractInput" },
  { name = "config", field = "config", type = "json_object", optional = true },
]

[crates.e2e.call.overrides.dart]
function = "extract"
options_type = "ExtractionConfig"

[crates.e2e.call.overrides.swift]
unnamed_arg_indices = [0, 1]

[crates.e2e.call.overrides.r]
function = "extract"
options_type = "ExtractionConfig"

[crates.e2e.package.swift]
name = "Xberg"

[crates.e2e.package.r]
name = "xberg"
"#;

fn config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn fixture() -> Fixture {
    Fixture {
        id: "api_extract_uri_async".to_string(),
        category: Some("contract".to_string()),
        description: "unified extract uses fixture input".to_string(),
        call: None,
        input: serde_json::json!({
            "kind": "uri",
            "uri": "pdf/fake_memo.pdf"
        }),
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
        source: "contract/api_extract_uri_async.json".to_string(),
        ..Fixture::default()
    }
}

fn group() -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![fixture()],
    }]
}

#[test]
fn dart_unified_extract_single_fixture_emits_input_arg() {
    let (e2e, resolved) = config();
    let files = DartE2eCodegen
        .generate(&group(), &e2e, &resolved, &[], &[])
        .expect("dart e2e generation succeeds");
    let content = &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("contract_test.dart"))
        .expect("contract_test.dart is emitted")
        .content;

    assert!(
        content.contains("final _input = await createExtractInputFromJson(json:"),
        "Dart must materialize the ExtractInput fixture JSON. Generated:\n{content}"
    );
    assert!(
        content.contains("XbergBridge.extract(_input"),
        "Dart extract call must pass input. Generated:\n{content}"
    );
    assert!(
        !content.contains("XbergBridge.extract(config: _config)"),
        "Dart extract call must not omit input. Generated:\n{content}"
    );
}

#[test]
fn swift_unified_extract_single_fixture_emits_input_json() {
    let (e2e, resolved) = config();
    let files = SwiftE2eCodegen
        .generate(&group(), &e2e, &resolved, &[], &[])
        .expect("swift e2e generation succeeds");
    let content = &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("ContractTests.swift"))
        .expect("ContractTests.swift is emitted")
        .content;

    assert!(
        content.contains("let result = try await Xberg.extract(\"{"),
        "Swift extract call must pass fixture input JSON. Generated:\n{content}"
    );
    assert!(
        content.contains("\\\"kind\\\":\\\"uri\\\"") && content.contains("\\\"uri\\\":\\\"pdf/fake_memo.pdf\\\""),
        "Swift input JSON must contain the fixture ExtractInput fields. Generated:\n{content}"
    );
    assert!(
        !content.contains("Xberg.extract([],"),
        "Swift extract call must not default input to an empty array. Generated:\n{content}"
    );
}

#[test]
fn r_unified_extract_single_fixture_emits_input_arg() {
    let (e2e, resolved) = config();
    let files = RCodegen
        .generate(&group(), &e2e, &resolved, &[], &[])
        .expect("R e2e generation succeeds");
    let content = &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("test_contract.R"))
        .expect("test_contract.R is emitted")
        .content;

    assert!(
        content.contains("extract(input = ExtractInput$from_json("),
        "R extract call must pass fixture input. Generated:\n{content}"
    );
    assert!(
        !content.contains("extract(config = "),
        "R extract call must not omit input. Generated:\n{content}"
    );
}
