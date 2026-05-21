//! Verifies the wasm e2e codegen auto-skips fixtures whose `input.language`
//! falls outside the `[crates.wasm].languages` static-compiled set.
//!
//! Pre-fix the override was only applied when `should_include_fixture` already
//! returned false for unrelated reasons. Since that helper does not inspect
//! `input.language`, fixtures targeting grammars absent from the wasm bundle
//! (e.g. `abl`) were emitted as live tests and failed at runtime against a
//! missing parser. This guard pins the auto-skip behaviour: fixtures with
//! `input.language` outside the configured list must render as `it.skip(...)`
//! with the canonical reason string, while fixtures targeting included
//! languages must still render as live tests.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::wasm::WasmCodegen;
use alef_e2e::fixture::{Fixture, FixtureGroup};

fn build_config() -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
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
        call: None,
        input: serde_json::json!({ "language": language }),
        mock_response: None,
        visitor: None,
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

    // The abl fixture must render as `it.skip(...)` with the canonical reason.
    assert!(
        body.contains("smoke_abl"),
        "abl fixture id must still appear in test file (as it.skip), got:\n{body}"
    );
    assert!(
        body.contains("language not in WASM's static-compiled set"),
        "abl fixture must carry the auto-skip reason, got:\n{body}"
    );

    // Heuristic: the abl block uses `it.skip(`. Find the slice around `smoke_abl`
    // and assert that `it.skip` precedes the python block (which should not be skipped).
    let abl_idx = body.find("smoke_abl").expect("smoke_abl present");
    let abl_prefix = &body[..abl_idx];
    let last_it_call = abl_prefix.rfind("it(").or_else(|| abl_prefix.rfind("it.skip("));
    assert!(
        last_it_call.is_some(),
        "expected an `it(` or `it.skip(` call before smoke_abl"
    );
    // The closest preceding it-call for abl must be it.skip.
    let last_skip = abl_prefix.rfind("it.skip(");
    let last_it = abl_prefix.rfind("it(");
    assert!(
        last_skip.is_some() && last_skip.unwrap() > last_it.unwrap_or(0),
        "smoke_abl must be wrapped in it.skip(...), got body:\n{body}"
    );

    // The python fixture must render as a live `it(` test, not it.skip.
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
    // Sanity check: when [crates.wasm].languages is empty (i.e. omitted), the
    // override must be inert and `smoke_abl` must render as a live test.
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
