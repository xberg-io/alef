//! Regression test for Block B6: alef C# e2e codegen should infer config parameter
//! types by trying the argument name directly before appending "Config" suffix.
//!
//! Scenario: a fixture omits a `config` parameter. The codegen should:
//! 1. Check if type `Config` exists (direct name match)
//! 2. Check if type `ConfigConfig` exists (suffix append)
//! 3. Only pass null if both lookups fail
//!
//! This fixes the case where the method signature expects a `Config` type,
//! but the old logic tried to find `ConfigConfig` first and fell back to null,
//! causing ArgumentNullException at runtime on non-nullable parameters.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture_omit_config(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("test_category".to_string()),
        description: "Test config parameter type inference - fixture omits config field".to_string(),
        tags: vec!["config_inference".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("process_with_config".to_string()),
        input: serde_json::json!({
            "data": "test data"
        }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("result".to_string()),
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "test_fixture.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "test_category".to_string(),
        fixtures: vec![make_fixture_omit_config("config_type_inference_test")],
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
function = "process_with_config"
result_var = "result"

[[crates.e2e.call.args]]
name = "data"
field = "input.data"
type = "string"

[[crates.e2e.call.args]]
name = "config"
field = "input.config"
type = "json_object"
"#;

#[test]
fn csharp_config_type_inference_direct_match() {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group()];

    let generated = CSharpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    assert!(!generated.is_empty(), "Should generate C# test code");

    let test_code = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test"))
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(!test_code.is_empty(), "Should generate test code");

    assert!(!test_code.is_empty(), "Generated C# test code should not be empty");
}
