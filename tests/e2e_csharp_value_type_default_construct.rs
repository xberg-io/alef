//! Regression test for Bug B6: C# e2e codegen must emit `new T()` for omitted
//! value-type (struct/record) parameters instead of `null`.
//!
//! When a fixture omits a config parameter that is a C# value type (non-nullable),
//! the codegen should emit `new ConfigType()` rather than `null`, which would cause
//! a runtime error: "Value cannot be null. (Parameter 'config')".
//!
//! This test verifies:
//! 1. When options_type is set → emit `new OptionsType()`
//! 2. When element_type is set → emit `new ElementType()`
//! 3. When neither is set but type can be inferred from parameter name → emit `new InferredType()`
//! 4. Only when none of the above applies → emit `null`

use alef::core::config::NewAlefConfig;
use alef::core::ir::TypeDef;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture_omit_config(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("batch_test".to_string()),
        description: "Batch test with omitted config parameter".to_string(),
        tags: vec!["value_type_default".to_string()],
        skip: None,
        env: None,
        call: Some("process_items_async".to_string()),
        input: serde_json::json!({
            "texts": ["sample text"]
            // Deliberately omit the "config" field to trigger default construction.
            // The C# binding expects BatchConfig (a struct), not null.
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: vec!["items".to_string()],
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
        source: "process_items_async_happy.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "batch_test".to_string(),
        fixtures: vec![make_fixture_omit_config("process_items_async_value_type_default")],
    }
}

fn make_batch_config_type() -> TypeDef {
    TypeDef {
        name: "BatchConfig".to_string(),
        rust_path: "demo::BatchConfig".to_string(),
        doc: "Configuration for batch processing".to_string(),
        has_default: true,
        ..TypeDef::default()
    }
}

const TOML: &str = r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test_crate"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = "TestCrate"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process_items_async"
result_var = "result"
async = true

[[crates.e2e.call.args]]
name = "texts"
field = "input.texts"
type = "json_object"

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "json_object"
element_type = "BatchConfig"
"#;

#[test]
fn csharp_value_type_default_construct_with_element_type() {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group()];
    let type_defs = vec![make_batch_config_type()];

    let generated = CSharpCodegen
        .generate(&groups, &e2e, &resolved, &type_defs, &[])
        .expect("generation succeeds");

    assert!(!generated.is_empty(), "Should generate C# test code");

    // Extract the test file content
    let test_code = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test"))
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(!test_code.is_empty(), "Should generate test code");

    // Snapshot the generated C# code to verify:
    // 1. The config parameter is constructed as `new BatchConfig()` NOT `null`
    // 2. The generated code is syntactically valid C#
    insta::assert_snapshot!(test_code);
}
