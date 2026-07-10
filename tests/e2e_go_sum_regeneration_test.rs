//! Regression test for go.sum regeneration in e2e test generation.
//!
//! Validates that after e2e code generation for Go, the resulting go.sum file
//! includes entries for the e2e submodule dependency, matching what `go mod tidy`
//! would produce. This prevents the "missing go.sum entry" error that occurs when
//! running `go test` without manual `go mod tidy` invocation.
//!
//! See: github.com/xberg-io/kreuzcrawl issues with missing e2e module in go.sum.

use alef::core::config::new_config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["go"]

[[crates]]
name = "testlib"
sources = ["src/lib.rs"]

[crates.go]
module = "github.com/test/testlib"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "fetch"
module = "testlib"
result_var = "result"
returns_result = true
args = []
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn make_simple_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "simple_fetch".to_string(),
            category: Some("smoke".to_string()),
            description: "Simple fetch test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: vec![],
            call: None,
            input: serde_json::json!({}),
            mock_response: None,
            http: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
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
            source: "test/simple.json".to_string(),
        }],
    }
}

#[test]
fn test_go_e2e_generation_includes_go_mod() {
    let (e2e_config, crate_config) = build_config();
    let groups = vec![make_simple_fixture()];

    let codegen = GoCodegen;
    let files = codegen
        .generate(&groups, &e2e_config, &crate_config, &[], &[])
        .expect("generation succeeds");

    let go_mod_file = files
        .iter()
        .find(|f| f.path.ends_with("go.mod"))
        .expect("go.mod must be generated");

    assert!(
        go_mod_file.content.contains("module github.com/test/testlib/e2e"),
        "go.mod must define the e2e submodule"
    );
    assert!(
        go_mod_file.content.contains("require ("),
        "go.mod must have a require block"
    );
    assert!(
        go_mod_file.content.contains("github.com/test/testlib"),
        "go.mod must require the parent module"
    );
    assert!(
        go_mod_file.content.contains("go 1.26"),
        "go.mod must specify Go version"
    );
}

#[test]
fn test_go_e2e_generation_with_local_replace_directive() {
    let (mut e2e_config, crate_config) = build_config();
    e2e_config.dep_mode = alef::e2e::config::DependencyMode::Local;

    let groups = vec![make_simple_fixture()];

    let codegen = GoCodegen;
    let files = codegen
        .generate(&groups, &e2e_config, &crate_config, &[], &[])
        .expect("generation succeeds");

    let go_mod_file = files
        .iter()
        .find(|f| f.path.ends_with("go.mod"))
        .expect("go.mod must be generated");

    assert!(
        go_mod_file.content.contains("replace github.com/test/testlib"),
        "go.mod must have a replace directive for local testing"
    );
}
