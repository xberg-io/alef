//! Regression coverage: `WasmCodegen::generate` must EMIT a visible placeholder suite when
//! every fixture in a category is excluded for wasm, rather than silently dropping the
//! category.
//!
//! Why this exists as a separate integration test rather than extending the unit tests in
//! `src/e2e/codegen/wasm/tests.rs`: the unit test
//! `render_wasm_excluded_category_emits_named_skip_cases_with_reasons` calls the renderer
//! function DIRECTLY. It proves the renderer produces good output, but never proves that
//! `generate` CALLS it — and the regression being guarded against lives entirely in that
//! call path. Neutralising the guard in `wasm.rs` (`if !group.fixtures.is_empty()` ->
//! `if false`) fully restores the original silent-drop bug and leaves all 7 unit tests
//! green. This test drives `generate` end to end so that regression fails loudly.
//!
//! Original defect: wasm emitted 229 of 283 fixtures — the entire `visitor` category (49
//! fixtures across 9 files) vanished with no file, no warning and no CI failure.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::wasm::WasmCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, SkipDirective};

const EXCLUDED_CATEGORY: &str = "visitor";
const EXCLUDED_FIXTURE: &str = "visitor_skip_heading";
const SKIP_REASON: &str = "WASM visitor bridge not yet exposed";

fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "demo-markup-rs"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "demo_markup"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]

[crates.e2e.call.overrides.wasm]
module = "@demo/markup-wasm"
function = "convert"
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn resolve(
    cfg: &NewAlefConfig,
) -> (
    alef::core::config::ResolvedCrateConfig,
    alef::core::config::e2e::E2eConfig,
) {
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

/// A category in which EVERY fixture is excluded for wasm — the condition that used to
/// make the whole category disappear silently.
fn fully_excluded_group() -> FixtureGroup {
    FixtureGroup {
        category: EXCLUDED_CATEGORY.to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: EXCLUDED_FIXTURE.to_string(),
            category: Some(EXCLUDED_CATEGORY.to_string()),
            description: "every fixture in this category is excluded for wasm".to_string(),
            tags: Vec::new(),
            skip: Some(SkipDirective {
                languages: vec!["wasm".to_string()],
                reason: Some(SKIP_REASON.to_string()),
            }),
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<h1>hi</h1>" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: Vec::new(),
            source: "visitor.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

#[test]
fn wasm_generate_emits_placeholder_for_a_fully_excluded_category() {
    let cfg = build_config();
    let (resolved, e2e) = resolve(&cfg);
    let groups = vec![fully_excluded_group()];

    let files = WasmCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("wasm generation succeeds");

    let combined = files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n");

    // The category must not vanish. This is the assertion the direct-renderer unit test
    // cannot make, because it never exercises `generate`.
    assert!(
        !files.is_empty(),
        "generate emitted NO files at all for a fully-excluded category — the category was silently dropped"
    );
    assert!(
        combined.contains(EXCLUDED_CATEGORY),
        "generated output must still name the excluded category {EXCLUDED_CATEGORY:?}; \
         a fully-excluded category was silently dropped. Output:\n{combined}"
    );
    assert!(
        combined.contains(EXCLUDED_FIXTURE),
        "generated output must name each excluded fixture ({EXCLUDED_FIXTURE:?}) so the \
         omission is visible. Output:\n{combined}"
    );
    assert!(
        combined.contains(SKIP_REASON),
        "generated output must surface the per-fixture skip reason ({SKIP_REASON:?}). \
         Output:\n{combined}"
    );
}
