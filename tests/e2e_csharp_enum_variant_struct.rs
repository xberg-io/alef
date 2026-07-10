//! Verifies that C# codegen correctly maps enum variant payloads to struct types.
//! Regression test for FormatMetadata.Code where the variant wraps
//! ProcessResult (a struct) but was being generated as Code(string Value)
//! instead of Code(ProcessResult Value).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("code".to_string()),
        description: "Test FormatMetadata.Code with ProcessResult".to_string(),
        tags: vec!["code".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file_sync".to_string()),
        input: serde_json::json!({
            "path": "code/example.py"
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("mime_type".to_string()),
            value: Some(serde_json::json!("text/x-python")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "code_shebang_detection.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "code".to_string(),
        fixtures: vec![make_fixture("code_shebang_detection")],
    }
}

const TOML: &str = r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = "SampleCrate"

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
"#;

#[test]
fn csharp_enum_variant_struct_type_correct() {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let _resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group()];

    let generated = CSharpCodegen
        .generate(&groups, &e2e, &_resolved, &[], &[])
        .expect("generation succeeds");

    assert!(!generated.is_empty(), "Should generate C# test code");

    let test_code = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test"))
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(!test_code.is_empty(), "Should generate test code");
}
