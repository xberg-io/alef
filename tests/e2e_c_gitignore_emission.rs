//! Verifies the C e2e codegen emits `e2e/c/.gitignore` covering the linked
//! `run_tests` binary, intermediate object files, and mock-server pipe
//! artifacts.
//!
//! Without the `.gitignore`, a developer who builds locally on macOS will
//! commit a Mach-O `run_tests` binary that fails Linux CI with `Exec format
//! error: ./run_tests` before the test suite ever runs.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::c::CCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> NewAlefConfig {
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

fn build_fixture() -> FixtureGroup {
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
fn c_codegen_emits_gitignore() {
    let cfg = build_config();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![build_fixture()];
    let files = CCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("C generation succeeds");

    let gitignore = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some(".gitignore"))
        .expect("`.gitignore` should be emitted alongside Makefile/main.c");

    let parent = gitignore.path.parent().expect("gitignore has a parent dir");
    assert_eq!(
        parent.file_name().and_then(|n| n.to_str()),
        Some("c"),
        ".gitignore must live directly under the C e2e output root"
    );

    let content = &gitignore.content;
    assert!(
        content.contains("run_tests"),
        ".gitignore must ignore the linked test binary `run_tests`. Got:\n{content}"
    );
    assert!(
        content.contains("*.o"),
        ".gitignore must ignore intermediate object files (`*.o`). Got:\n{content}"
    );
    assert!(
        content.contains("mock_server.stdout"),
        ".gitignore must ignore mock-server stdout pipe. Got:\n{content}"
    );
    assert!(
        content.contains("mock_server.stdin"),
        ".gitignore must ignore mock-server stdin pipe. Got:\n{content}"
    );
}
