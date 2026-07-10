//! Regression test: verifies that TypeScript e2e code generation produces
//! oxfmt-canonical output (double quotes, tab indentation, trailing semicolons)
//! so that running oxfmt on the emitted test files produces no diffs (regression
//! against generated packages with strict formatting checks).

use alef::core::config::NewAlefConfig;
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

[crates.e2e.call]
function = "convert"
module = "mylib"
result_var = "result"
result_is_simple = true
async = true
returns_result = false
args = [
  { name = "html", field = "input.html", type = "string" },
  { name = "options", field = "input.options", type = "json_object", optional = true },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn simple_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "convert_simple_html".to_string(),
            category: Some("smoke".to_string()),
            description: "converts simple HTML".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "html": "<p>Hello world</p>",
                "options": null
            }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
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
            source: "smoke/convert_simple_html.json".to_string(),
            http: None,
        }],
    }
}

/// Verify that emitted TypeScript test files use oxfmt canonical formatting:
/// - double quotes (not single quotes)
/// - tab indentation (not spaces)
/// - trailing semicolons on statements
///
/// This is verified by:
/// 1. Generating the test file
/// 2. Checking for indicators of correct formatting (double quotes in imports, describe blocks)
/// 3. Checking for non-canonical patterns (leading spaces before indentation)
#[test]
fn typescript_emitted_code_is_oxfmt_canonical() {
    let (e2e, resolved) = build_config();
    let groups = vec![simple_fixture_group()];
    let files = TypeScriptCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke.test.ts"))
        .expect("TypeScript smoke.test.ts is emitted");

    let body = &test_file.content;

    assert!(
        body.contains("import { describe, expect, it } from \"vitest\";"),
        "import vitest should use double quotes, got:\n{body}"
    );

    assert!(
        body.contains("describe(\"smoke\""),
        "describe block should use double quotes, got:\n{body}"
    );

    assert!(
        body.contains("it(\"convert_simple_html"),
        "it block should use double quotes, got:\n{body}"
    );

    assert!(
        !body.contains("from 'vitest'"),
        "imports should not use single quotes, got:\n{body}"
    );

    let has_tab_indentation = body
        .split('\n')
        .any(|line| line.starts_with('\t') && (line.contains("expect") || line.contains("const")));
    assert!(
        has_tab_indentation,
        "test body should use tab indentation, got:\n{body}"
    );

    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("vitest.config.ts"))
        .expect("vitest.config.ts is emitted");

    let config_body = &config_file.content;

    assert!(
        config_body.contains("import { defineConfig } from \"vitest/config\";"),
        "config import should use double quotes, got:\n{config_body}"
    );

    assert!(
        config_body.contains("\ttest: {"),
        "config should use tab indentation, got:\n{config_body}"
    );

    assert!(
        !config_body.contains("from 'vitest/config'"),
        "config import should not use single quotes, got:\n{config_body}"
    );
}
