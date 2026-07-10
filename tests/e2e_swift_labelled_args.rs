//! Test for Swift e2e function call argument labelling.
//!
//! The Swift backend must emit labelled arguments in free-function calls to match
//! the high-level binding signatures. For example:
//!   try SampleLanguagePack.process(source: "x", config: configObj)
//! not:
//!   try process("x", configObj)

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_swift_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "SampleLanguagePack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "SampleLanguagePack"
result_var = "result"
async = false
args = [
  { name = "source", field = "input.source", type = "string" },
  { name = "config", field = "input.config", type = "json_object" },
]

[crates.e2e.package.swift]
name = "SampleLanguagePack"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

#[test]
fn swift_emits_labelled_arguments_on_free_function_calls() {
    let (e2e_config, crate_config) = build_swift_config();

    let fixture = Fixture {
        id: "test_process_basic".to_string(),
        category: Some("smoke".to_string()),
        description: "Test fixture with labelled args".to_string(),
        call: None,
        skip: None,
        input: serde_json::json!({
            "source": "x = 1",
            "config": { "language": "python" }
        }),
        mock_response: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        tags: vec![],
        env: None,
        setup: Vec::new(),
        visitor: None,
        source: "smoke/test_process_basic.json".to_string(),
        http: None,
    };

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];

    let files = SwiftE2eCodegen
        .generate(&groups, &e2e_config, &crate_config, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("SmokeTests.swift"))
        .expect("SmokeTests.swift should be generated");

    let content = &test_file.content;

    assert!(
        content.contains("source:") && content.contains("config:"),
        "Swift function call must use argument labels. Generated code:\n{content}"
    );

    let has_qualified_source_label =
        content.contains("source:") && !content.contains("try SampleLanguagePack.process(\"");
    assert!(
        has_qualified_source_label,
        "Swift call must qualify free-function with module name and use argument labels"
    );
}

#[test]
fn swift_qualifies_free_function_calls_with_module_name() {
    let toml_src = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "SampleLanguagePack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "languageCount"
module = "SampleLanguagePack"
result_var = "result"
async = false
args = []

[crates.e2e.package.swift]
name = "SampleLanguagePack"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let crate_config = cfg.resolve().expect("resolves").remove(0);

    let fixture = Fixture {
        id: "test_language_count".to_string(),
        category: Some("registry".to_string()),
        description: "Test languageCount with module qualification".to_string(),
        call: None,
        skip: None,
        input: serde_json::json!({}),
        mock_response: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "greater_than".to_string(),
            field: None,
            value: Some(serde_json::Value::Number(0.into())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        tags: vec![],
        env: None,
        setup: Vec::new(),
        visitor: None,
        source: "registry/test_language_count.json".to_string(),
        http: None,
    };

    let groups = vec![FixtureGroup {
        category: "registry".to_string(),
        fixtures: vec![fixture],
    }];

    let files = SwiftE2eCodegen
        .generate(&groups, &e2e, &crate_config, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("RegistryTests.swift"))
        .expect("RegistryTests.swift should be generated");

    let content = &test_file.content;

    assert!(
        content.contains("SampleLanguagePack.languageCount()"),
        "Swift free-function call must be qualified with module name. Generated code:\n{content}"
    );
}
