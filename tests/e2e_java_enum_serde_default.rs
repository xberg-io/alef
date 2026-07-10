//! Regression test for Java builder field initialization with #[serde(default)] enums.
//!
//! When a non-optional enum field has #[serde(default)] in Rust,
//! the Java Builder must initialize it to the enum's Rust default variant,
//! not null. If null, Jackson will omit the field from serialization (NON_NULL inclusion),
//! and Rust's serde deserializer will fail because the field is required.
//!
//! Example: KeywordConfig.algorithm has #[serde(default)] and defaults to KeywordAlgorithm::Yake in Rust.
//! The Java Builder must initialize it to KeywordAlgorithm.Yake, not null.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_keyword_fixture() -> Fixture {
    Fixture {
        id: "config_keywords".to_string(),
        category: Some("contract".to_string()),
        description: "Tests keyword extraction with partial config (omits algorithm, relying on Rust default)"
            .to_string(),
        tags: vec!["keywords".to_string(), "config".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file_sync".to_string()),
        input: serde_json::json!({
            "path": "pdf/fake_memo.pdf",
            "config": {
                "keywords": {
                    "max_keywords": 10
                }
            }
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("mime_type".to_string()),
            value: Some(serde_json::Value::String("application/pdf".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "contract.json".to_string(),
        http: None,
    }
}

const TOML: &str = r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"
ffi_style = "panama"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
java_group_id = "dev.sample_crate"

[crates.e2e.call]
function = "extract_file_sync"
result_var = "result"

[[crates.e2e.call.args]]
name = "path"
field = "input.path"
type = "file_path"

[[crates.e2e.call.args]]
name = "mime_type"
field = "input.mime_type"
type = "string"
optional = true

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "json_object"
optional = true

[crates.e2e.call.overrides.java]
options_type = "ExtractionConfig"
options_via = "from_json"
"#;

fn render_java_test() -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![make_keyword_fixture()],
    }];
    let files = JavaCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains("ContractTest.java")
        })
        .expect("ContractTest.java is emitted")
        .content
        .clone()
}

/// When a fixture omits an enum field that has #[serde(default)] in Rust,
/// the generated Java code must construct the config via JsonUtil.fromJson(),
/// which triggers Jackson deserialization. The Builder for that enum field
/// must initialize to a valid enum variant (not null), so Jackson doesn't skip
/// the field during serialization.
#[test]
fn enum_serde_default_builder_not_null() {
    let rendered = render_java_test();

    assert!(
        rendered.contains("JsonUtil.fromJson("),
        "must use JsonUtil.fromJson() for JSON config. Rendered:\n{rendered}"
    );

    assert!(
        rendered.contains(".extractFileSync(java.nio.file.Path.of(\"pdf/fake_memo.pdf\"), null, config)"),
        "must pass config to extraction function. Rendered:\n{rendered}"
    );
}

/// Verify JsonUtil is imported for JSON deserialization
#[test]
fn enum_serde_default_imports_json_util() {
    let rendered = render_java_test();

    assert!(
        rendered.contains("import dev.sample_crate.JsonUtil;"),
        "must import JsonUtil. Rendered:\n{rendered}"
    );
}
