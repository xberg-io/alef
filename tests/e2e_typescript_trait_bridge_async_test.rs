//! Regression test for Block B11: trait bridge tests must be async and await calls.
//!
//! When a test fixture uses arg_type="test_backend" (trait bridge), the generated
//! TypeScript test must be `async` and the call must be `await`ed. This ensures that
//! any tokio tasks spawned internally by the trait bridge don't block process shutdown.

use alef::core::config::NewAlefConfig;
use alef::core::config::e2e::ArgMapping;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::typescript::TypeScriptCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["node"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[[crates.trait_bridges]]
trait_name = "MyBackend"
super_trait = "Plugin"
register_fn = "register_my_backend"
unregister_fn = "unregister_my_backend"

[crates.e2e.call]
function = "register_my_backend"
module = "mylib"
args = [
  { name = "backend", field = "input.backend", arg_type = "test_backend", trait = "MyBackend" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

#[test]
fn trait_bridge_tests_are_async_and_await_calls() {
    let (e2e_config, resolved_config) = build_config();

    let fixture = Fixture {
        id: "register_my_backend_trait_bridge".to_string(),
        category: Some("plugin_api".to_string()),
        description: "register_my_backend: trait bridge".to_string(),
        tags: vec!["trait-bridge".to_string()],
        skip: None,
        env: None,
        call: Some("register_my_backend".to_string()),
        input: serde_json::json!({
            "backend": {
                "type": "test",
                "name": "test-backend"
            }
        }),
        mock_response: None,
        visitor: None,
        args: vec![ArgMapping {
            name: "backend".to_string(),
            field: "input.backend".to_string(),
            arg_type: "test_backend".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("MyBackend".to_string()),
        }],
        assertion_recipes: vec![],
        http: None,
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
        source: "tests/e2e_typescript_trait_bridge_async_test.rs".to_string(),
    };

    let fixtures = vec![FixtureGroup {
        category: "plugin_api".to_string(),
        fixtures: vec![fixture],
    }];

    let codegen = TypeScriptCodegen;
    let generated = codegen
        .generate(&fixtures, &e2e_config, &resolved_config, &[], &[])
        .expect("generates without error");

    // Find the test file among generated files
    let test_file = generated
        .iter()
        .find(|f| f.path.to_string_lossy().contains("tests/"))
        .expect("test file exists");

    let content = &test_file.content;

    // Verify the test function is async
    assert!(
        content.contains("it(\"register_my_backend_trait_bridge"),
        "test name not found"
    );
    assert!(
        content.contains("async ()"),
        "test function should be async when it contains trait bridge args"
    );

    // Verify the call is awaited
    assert!(
        content.contains("await registerMyBackend"),
        "call to register function should be awaited"
    );

    // Verify the test stub class exists
    assert!(
        content.contains("_TestStub_register_my_backend_trait_bridge"),
        "test stub class should be generated"
    );
}
