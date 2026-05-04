//! Verifies the wasm e2e codegen emits real `extractFile`-style test cases for
//! non-HTTP fixtures (the extract_file / extract_bytes / chunk / detect surface),
//! not just HTTP-server fixtures.
//!
//! Pre-0.13.4 the wasm codegen filtered `f.http.is_some()` and dropped every
//! function-call fixture, leaving e2e/wasm/ with only globalSetup + package.json
//! and zero test files. Once those orphans were swept, the wasm e2e suite ran
//! zero tests against the binding's core async API. This regression guards
//! against that: a fixture with no `http` block must materialise as an
//! invocation of the binding under test.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::wasm::WasmCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
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
function = "extract_file"
module = "mylib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "path", field = "input.path", type = "file_path" },
  { name = "mime_type", field = "input.mime_type", type = "string", optional = true },
]

[crates.e2e.call.overrides.wasm]
options_type = "WasmExtractionConfig"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn smoke_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "extract a small pdf".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "path": "pdf/fake_memo.pdf" }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("content".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "smoke/smoke_basic.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn wasm_codegen_emits_extract_file_call_for_non_http_fixture() {
    let (e2e, resolved) = build_config();
    let groups = vec![smoke_fixture()];
    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");

    let smoke = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke.test.ts"))
        .expect("wasm smoke test file is emitted for non-HTTP fixture");

    let body = &smoke.content;

    // Function-call fixture is rendered, not a stub or skip.
    assert!(body.contains("extractFile"), "expected extractFile call, got:\n{body}");
    assert!(
        body.contains("await extractFile"),
        "extract_file is async — generated call must await, got:\n{body}"
    );
    // Imports the wasm package, not the npm node package.
    // The WASM codegen uses dynamic `await import('mylib')` since v0.14.5.
    let imports_wasm_pkg = body.contains("from 'mylib'")
        || body.contains("from \"mylib\"")
        || body.contains("import('mylib')")
        || body.contains("import(\"mylib\")");
    assert!(
        imports_wasm_pkg,
        "expected import from wasm package 'mylib', got:\n{body}"
    );
    // describe() block is for the right category and contains the fixture id.
    assert!(body.contains("describe('smoke'"), "missing describe block");
    assert!(body.contains("smoke_basic"), "missing fixture id");
}

#[test]
fn wasm_codegen_emits_setup_ts_when_file_path_args_are_used() {
    let (e2e, resolved) = build_config();
    let groups = vec![smoke_fixture()];
    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");

    // setup.ts must be emitted so that vitest chdir's to test_documents
    // before tests run, otherwise relative file paths fail to resolve.
    assert!(
        files.iter().any(|f| f.path.ends_with("setup.ts")),
        "setup.ts must be generated when any active fixture has a file_path arg"
    );

    // vitest config wires it up.
    let vitest = files
        .iter()
        .find(|f| f.path.ends_with("vitest.config.ts"))
        .expect("vitest config emitted");
    assert!(
        vitest.content.contains("setupFiles: ['./setup.ts']"),
        "vitest.config.ts must wire setupFiles when setup.ts is generated, got:\n{}",
        vitest.content
    );
}

#[test]
fn wasm_codegen_skips_globalsetup_when_no_http_fixtures() {
    let (e2e, resolved) = build_config();
    let groups = vec![smoke_fixture()];
    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");

    // Without any HTTP fixtures, we must not emit globalSetup.ts (which spawns
    // the rust mock-server). vitest config must also not reference it.
    assert!(
        !files.iter().any(|f| f.path.ends_with("globalSetup.ts")),
        "globalSetup.ts must not be generated when no HTTP fixtures are in scope"
    );
    let vitest = files
        .iter()
        .find(|f| f.path.ends_with("vitest.config.ts"))
        .expect("vitest config emitted");
    assert!(
        !vitest.content.contains("globalSetup"),
        "vitest.config.ts must not wire globalSetup when none is generated, got:\n{}",
        vitest.content
    );
}
