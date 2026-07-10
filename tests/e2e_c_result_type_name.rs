//! Verifies the C e2e codegen derives `result_type_name` from the base call
//! function name, not the C-overridden prefixed function name.
//!
//! Without the fix, a C override of `function = "htm_convert"` with prefix `htm`
//! produces `HTMHtmConvert*` — the prefix is doubled.  The fallback must use
//! `call.function` (the base, un-prefixed name) so it produces `HTMConvert*`,
//! which is at least not self-contradictory and matches the `<prefix><Base>` pattern.
//! When the correct type differs, users add an explicit `result_type` override.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::c::CCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn resolve_one(
    cfg: &NewAlefConfig,
) -> (
    alef::core::config::ResolvedCrateConfig,
    alef::core::config::e2e::E2eConfig,
) {
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

fn build_c_config_with_prefix_override() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "demo-markup-rs"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "htm"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]

[crates.e2e.call.overrides.c]
header = "demo_markup.h"
function = "htm_convert"
prefix = "htm"
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_simple_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "basic conversion".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<p>hi</p>" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn c_result_type_does_not_double_prefix() {
    let cfg = build_c_config_with_prefix_override();
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_simple_fixture()];
    let files = CCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("C generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_smoke.c"))
        .expect("test_smoke.c should be emitted");
    let content = &test_file.content;
    assert!(
        !content.contains("HTMHtmConvert"),
        "result type must not double the prefix (HTMHtmConvert found). Content:\n{content}"
    );
    assert!(
        content.contains("HTMConvert"),
        "result type should be HTMConvert (base function name in PascalCase). Content:\n{content}"
    );
}

#[test]
fn c_result_type_explicit_override_wins() {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "demo-markup-rs"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "htm"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]

[crates.e2e.call.overrides.c]
header = "demo_markup.h"
function = "htm_convert"
prefix = "htm"
result_type = "ConversionResult"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let (resolved, e2e) = resolve_one(&cfg);
    let groups = vec![build_simple_fixture()];
    let files = CCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("C generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("test_smoke.c"))
        .expect("test_smoke.c should be emitted");
    let content = &test_file.content;
    assert!(
        content.contains("HTMConversionResult"),
        "explicit result_type = 'ConversionResult' must produce HTMConversionResult. Content:\n{content}"
    );
    assert!(
        !content.contains("HTMHtmConvert"),
        "doubled prefix must not appear. Content:\n{content}"
    );
}
