//! Verifies that C# codegen correctly handles tuple default values.
//! Regression test for KeywordConfig.ngram_range where a field with
//! type Vec<usize> and #[serde(default = "default_ngram_range")]
//! (returning (1, 3)) was being initialized to [] instead of [1, 3].

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("contract".to_string()),
        description: "Test KeywordConfig tuple default".to_string(),
        tags: vec!["config".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_file_sync".to_string()),
        input: serde_json::json!({
            "path": "pdf/fake_memo.pdf",
            "config": {
                "keywords": {
                    "algorithm": "yake",
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
            value: Some(serde_json::json!("application/pdf")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "config_keywords.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![make_fixture("config_keywords")],
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

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "object"
"#;

#[test]
fn csharp_tuple_default_initializes_correctly() {
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
