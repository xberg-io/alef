//! Verifies that Java e2e codegen correctly handles json_object args
//! with values, ensuring proper deserialization via JsonUtil.fromJson().
//!
//! When a fixture provides a config JSON value for a json_object arg,
//! and the options_type resolves (either explicitly or via fallback from another language),
//! the generated Java code MUST use `JsonUtil.fromJson()` to construct the config,
//! not pass null or raw JSON literals.
//!
//! Bug scenario: If options_type doesn't properly fall back to other languages' overrides,
//! the code would emit raw JSON (via json_to_java), which fails for complex config types.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_extract_fixture() -> Fixture {
    Fixture {
        id: "extract_with_config".to_string(),
        category: Some("contract".to_string()),
        description: "Tests extraction with config provided".to_string(),
        tags: vec!["config".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file_sync".to_string()),
        input: serde_json::json!({
            "path": "pdf/test.pdf",
            "config": {
                "keywords": {
                    "algorithm": "yake",
                    "max_keywords": 5
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

/// with neutral ones.
/// Alef config with a json_object arg (config) that doesn't have an explicit
/// Java options_type but DOES have a C# options_type that Java should inherit.
const TOML: &str = r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"

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

[crates.e2e.call.overrides.csharp]
options_type = "ExtractionConfig"
options_via = "from_json"

# No explicit Java override — should inherit the options type from C#
"#;

fn render_java_test() -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![make_extract_fixture()],
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

/// When a fixture provides a config value for a json_object arg,
/// and options_type is resolved (even via fallback), the generated code must
/// construct it via JsonUtil.fromJson() with the resolved type.
#[test]
fn json_object_arg_with_value_uses_from_json() {
    let rendered = render_java_test();

    assert!(
        rendered.contains("JsonUtil.fromJson(") && rendered.contains("ExtractionConfig.class"),
        "must use JsonUtil.fromJson(..., ExtractionConfig.class) to construct config from JSON fixture value. Rendered:\n{rendered}"
    );

    assert!(
        rendered.contains(".extractFileSync(java.nio.file.Path.of(\"pdf/test.pdf\"), null, config)"),
        "must pass the constructed config variable to the function. Rendered:\n{rendered}"
    );
}

/// JsonUtil must be imported when constructing configs from JSON.
#[test]
fn json_object_arg_with_value_imports_json_util() {
    let rendered = render_java_test();

    assert!(
        rendered.contains("import dev.sample_crate.JsonUtil;"),
        "must import JsonUtil when deserializing config from JSON. Rendered:\n{rendered}"
    );
}
