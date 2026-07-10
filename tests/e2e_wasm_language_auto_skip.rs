//! Verifies the wasm e2e codegen filters fixtures whose `input.language`
//! falls outside the `[crates.wasm].languages` static-compiled set.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::wasm::WasmCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.wasm]
languages = ["python"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "parse"
module = "mylib"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "language", field = "input.language", type = "string" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture(id: &str, language: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: format!("parse {language}"),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "language": language }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: Vec::new(),
        source: format!("smoke/{id}.json"),
        http: None,
    }
}

#[test]
fn wasm_codegen_auto_skips_fixtures_outside_static_language_set() {
    let (e2e, resolved) = build_config();
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture("smoke_python", "python"), fixture("smoke_abl", "abl")],
    }];

    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let smoke = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke.test.ts"))
        .expect("smoke test file emitted");
    let body = &smoke.content;

    assert!(
        !body.contains("smoke_abl"),
        "abl fixture id must be omitted when outside the WASM static language set, got:\n{body}"
    );
    assert!(
        !body.contains("language not in WASM's static-compiled set"),
        "omitted abl fixture should not emit a skip reason, got:\n{body}"
    );

    let py_idx = body.find("smoke_python").expect("smoke_python present");
    let py_prefix = &body[..py_idx];
    let py_last_it = py_prefix.rfind("it(").unwrap_or(0);
    let py_last_skip = py_prefix.rfind("it.skip(").unwrap_or(0);
    assert!(
        py_last_it > py_last_skip,
        "smoke_python must render as a live `it(` test, not it.skip — body:\n{body}"
    );
}

#[test]
fn wasm_codegen_does_not_filter_when_languages_unset() {
    let toml_src = r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "parse"
module = "mylib"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "language", field = "input.language", type = "string" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture("smoke_abl", "abl")],
    }];

    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    let smoke = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke.test.ts"))
        .expect("smoke test file emitted");
    let body = &smoke.content;

    assert!(
        !body.contains("language not in WASM's static-compiled set"),
        "must not auto-skip when [crates.wasm].languages is unset, got:\n{body}"
    );
}
