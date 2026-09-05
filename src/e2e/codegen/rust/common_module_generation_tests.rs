//! `tests/common.rs` (and its `runtime()` helper) must be generated whenever ANY test in the
//! crate is async — not only when a mock server is also involved.
//!
//! ~keep `common::runtime()` now backs every async test's shared-runtime wrapper, including a
//! plain `[e2e.call] async = true` fixture with no `mock_response`/`http`. Gating `common.rs`'s
//! generation on `needs_mock_server` (as it was before this fix) left such a crate emitting
//! `common::runtime().block_on(...)` with no `common` module to resolve it against: E0433 at
//! `cargo test`, not at generation time.

use super::RustE2eCodegen;
use crate::core::config::NewAlefConfig;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::fixture::{Fixture, FixtureGroup};

fn config(text: &str) -> (crate::e2e::config::E2eConfig, crate::core::config::ResolvedCrateConfig) {
    let parsed: NewAlefConfig = toml::from_str(text).expect("e2e config must parse");
    let e2e = parsed.crates[0].e2e.clone().expect("e2e config present");
    let resolved = parsed.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn lone_group(fixture: serde_json::Value) -> Vec<FixtureGroup> {
    let fixture: Fixture = serde_json::from_value(fixture).expect("fixture must parse");
    assert!(
        !fixture.needs_mock_server(),
        "the whole point of these fixtures is that they need no mock server"
    );
    vec![FixtureGroup {
        category: "generated".to_string(),
        fixtures: vec![fixture],
    }]
}

fn process_fixture() -> serde_json::Value {
    serde_json::json!({
        "id": "process_it",
        "description": "process something",
        "input": null,
        "assertions": []
    })
}

const ASYNC_NO_MOCK_CONFIG: &str = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "sample_core"
result_var = "result"
async = true
"#;

const SYNC_NO_MOCK_CONFIG: &str = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "sample_core"
result_var = "result"
"#;

#[test]
fn async_only_crate_with_no_mock_server_still_generates_the_shared_runtime_module() {
    let (e2e, resolved) = config(ASYNC_NO_MOCK_CONFIG);
    let files = RustE2eCodegen
        .generate(&lone_group(process_fixture()), &e2e, &resolved, &[], &[], &[], &[])
        .expect("rust e2e crate generates");

    let paths: Vec<String> = files.iter().map(|f| f.path.display().to_string()).collect();
    let common = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == "common.rs"))
        .unwrap_or_else(|| panic!("tests/common.rs must be generated for an async-only crate; got: {paths:?}"));
    assert!(
        common
            .content
            .contains("pub fn runtime() -> &'static tokio::runtime::Runtime"),
        "{}",
        common.content
    );

    let test_body = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == "generated_test.rs"))
        .expect("generated_test.rs must exist");
    assert!(test_body.content.contains("mod common;"), "{}", test_body.content);
    assert!(
        test_body.content.contains("common::runtime().block_on(async {"),
        "{}",
        test_body.content
    );
    assert!(!test_body.content.contains("#[tokio::test]"), "{}", test_body.content);
}

/// Negative control: a fully synchronous crate with no async call and no mock server needs no
/// shared runtime at all, so `tests/common.rs` must not be generated for it.
#[test]
fn fully_sync_crate_with_no_mock_server_generates_no_shared_runtime_module() {
    let (e2e, resolved) = config(SYNC_NO_MOCK_CONFIG);
    let files = RustE2eCodegen
        .generate(&lone_group(process_fixture()), &e2e, &resolved, &[], &[], &[], &[])
        .expect("rust e2e crate generates");

    assert!(
        !files
            .iter()
            .any(|file| file.path.file_name().is_some_and(|name| name == "common.rs")),
        "a fully synchronous crate with no mock server must not generate tests/common.rs"
    );

    let test_body = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == "generated_test.rs"))
        .expect("generated_test.rs must exist");
    assert!(!test_body.content.contains("mod common;"), "{}", test_body.content);
    assert!(
        !test_body.content.contains("common::runtime()"),
        "{}",
        test_body.content
    );
}
