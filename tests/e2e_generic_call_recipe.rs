//! Regression tests for generic e2e call recipe resolution.

use alef::core::config::NewAlefConfig;
use alef::core::ir::TypeDef;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["rust", "dart"]

[[crates]]
name = "sample_recipe"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_recipe"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process_sample"
module = "sample_recipe"
result_var = "result"
options_type = "SampleSettings"
args = [
  { name = "settings", field = "input.settings", type = "json_object", optional = true },
]
"#;

fn config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn fixture() -> FixtureGroup {
    FixtureGroup {
        category: "sample".to_string(),
        fixtures: vec![Fixture {
            id: "sample_default_settings".to_string(),
            category: Some("sample".to_string()),
            description: "sample default settings".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({}),
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
            source: "sample.json".to_string(),
            http: None,
        }],
    }
}

fn sample_settings_type() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "SampleSettings".to_string(),
        has_default: true,
        ..TypeDef::default()
    }]
}

#[test]
fn rust_uses_call_level_options_type_for_json_object_annotation() {
    let (e2e, resolved) = config();
    let groups = vec![fixture()];
    let type_defs = sample_settings_type();
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &type_defs, &[])
        .expect("generation succeeds");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("sample_test.rs"))
        .expect("sample_test.rs is emitted")
        .content
        .clone();

    assert!(
        content.contains("use sample_recipe::SampleSettings;"),
        "call-level options_type must be imported for Rust annotations. Rendered:\n{content}"
    );
    assert!(
        content.contains("let settings: SampleSettings = Default::default();"),
        "optional json_object default must use the configured call-level type. Rendered:\n{content}"
    );
}

#[test]
fn dart_materializes_absent_optional_config_from_type_default() {
    let (e2e, resolved) = config();
    let groups = vec![fixture()];
    let type_defs = sample_settings_type();
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &type_defs, &[])
        .expect("generation succeeds");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("sample_test.dart"))
        .expect("sample_test.dart is emitted")
        .content
        .clone();

    assert!(
        content.contains("final _settings = await createSampleSettingsFromJson(json: '{}');"),
        "Dart must materialize default config from configured options_type + TypeDef.has_default. Rendered:\n{content}"
    );
    assert!(
        content.contains("processSample(settings: _settings)"),
        "Dart call must pass the materialized settings arg. Rendered:\n{content}"
    );
}
