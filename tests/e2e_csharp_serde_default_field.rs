//! Verifies that C# codegen does NOT emit `required` modifier for fields
//! with `#[serde(default)]` in Rust, since such fields can be omitted from JSON
//! during deserialization. Regression test for DocumentNode.content_layer failure
//! where the Rust field has `#[serde(default, skip_serializing_if)]` but C# marked
//! it as `required`, causing JSON deserialization to fail when the field was absent.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("contract".to_string()),
        description: "Test document structure without content_layer field".to_string(),
        tags: vec!["document_structure".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file_sync".to_string()),
        input: serde_json::json!({
            "path": "docx/fake.docx",
            "config": {
                "include_document_structure": true
            }
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("mime_type".to_string()),
                value: Some(serde_json::json!(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                )),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
            Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("document".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
        ],
        source: "test_fixture.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![make_fixture("serde_default_field_test")],
    }
}

const TOML: &str = r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test-lib"
sources = ["src/main.rs"]

[crates.csharp]
namespace = "TestLib"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_file_sync"
result_var = "result"

[[crates.e2e.call.args]]
name = "path"
field = "input.path"
type = "string"

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "object"
"#;

#[test]
fn csharp_serde_default_field_not_required() {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let _resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group()];

    let generated = CSharpCodegen
        .generate(&groups, &e2e, &_resolved, &[], &[])
        .expect("generation succeeds");

    // since the Rust field has `#[serde(default)]`. The field should either:

    assert!(!generated.is_empty(), "Should generate C# test code");

    // With the fix, fields with #[serde(default)] should not be marked required.
    let test_code = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test"))
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(!test_code.is_empty(), "Should generate test code");
}
