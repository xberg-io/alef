//! Regression for #405: the brew (Homebrew CLI) e2e harness's auto-spawned mock-server binary
//! path hard-coded `../rust/target/release/mock-server` as `MOCK_SERVER_BIN`'s only fallback,
//! which breaks under `CARGO_TARGET_DIR`/a `.cargo/config.toml` `build.target-dir` override.
//! The runner (`alef test-apps run`) now exports the resolved absolute path as
//! `ALEF_E2E_MOCK_SERVER`; `run_tests.sh` must read it (after any manually-set `MOCK_SERVER_BIN`
//! override) before falling back to the historical relative join.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::brew::BrewCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["brew"]

[crates.e2e.call]
function = "scrape"
module = "testlib"
result_var = "result"
args = [
  { name = "url", field = "url", type = "mock_url" },
]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "scrape_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "Basic scrape".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({"url": "/page"}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
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
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }
}

#[test]
fn run_tests_sh_mock_server_bin_prefers_alef_e2e_mock_server_env_var() {
    let cfg = build_config();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![build_fixture()];

    let files = BrewCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("brew generation succeeds");

    let run_tests = files
        .iter()
        .find(|f| f.path.ends_with("run_tests.sh"))
        .expect("run_tests.sh should be emitted");
    let content = &run_tests.content;

    assert!(
        content.contains(
            r#"MOCK_SERVER_BIN="${MOCK_SERVER_BIN:-${ALEF_E2E_MOCK_SERVER:-../rust/target/release/mock-server}}""#
        ),
        "run_tests.sh should read ALEF_E2E_MOCK_SERVER before falling back to the historical \
         literal, while still honoring a manually-set MOCK_SERVER_BIN override. Rendered:\n{content}"
    );
}
