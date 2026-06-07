//! Verifies the Swift e2e codegen coalesces `.toString().count` into a non-optional
//! when the accessor chain crosses an optional field.
//!
//! Regression: alef 0.15.57 emitted
//!     XCTAssertGreaterThan(result.markdown?.content.count, 0, ...)
//! which Swift rejects because `result.markdown()?.content().toString().count` is
//! `UInt?` and cannot be compared to the integer literal `0`. The fix
//! emits `(result.markdown()?.content().toString().count ?? 0)` so the comparison
//! typechecks. Symmetric fix applies to `is_empty`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn make_fixture(id: &str, assertion_type: &str, field: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "optional-chain assertion fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({}),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn make_group(id: &str, assertion_type: &str, field: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture(id, assertion_type, field)],
    }
}

fn render_swift(fixture_id: &str, assertion_type: &str, field: &str) -> Vec<alef::core::backend::GeneratedFile> {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo_crawler"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_optional = ["markdown"]

[crates.e2e.call]
function = "scrape"
module = "demo_crawler"
result_var = "result"
async = true

[[crates.e2e.call.args]]
name = "url"
field = "input.url"
type = "string"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id, assertion_type, field)];
    SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

fn smoke_test_content(files: &[alef::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted")
        .content
        .clone()
}

#[test]
fn not_empty_optional_chain_coalesces_len_to_int() {
    let files = render_swift("smoke_markdown_content", "not_empty", "markdown.content");
    let rendered = smoke_test_content(&files);
    assert!(
        rendered.contains("(result.markdown()?.content().toString().count ?? 0)"),
        "not_empty over an optional chain must coalesce `.toString().count` via `?? 0`. \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.markdown()?.content().toString().count, 0"),
        "must not emit the bare `?.chain.toString().count, 0` pattern that produces a \
         `UInt? vs Int` compile error. Rendered:\n{rendered}"
    );
}

#[test]
fn is_empty_optional_chain_coalesces_len_to_int() {
    let files = render_swift("smoke_markdown_empty", "is_empty", "markdown.content");
    let rendered = smoke_test_content(&files);
    assert!(
        rendered.contains("(result.markdown()?.content().toString().count ?? 0)"),
        "is_empty over an optional chain must coalesce `.toString().count` via `?? 0`. \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.markdown()?.content().toString().count, 0"),
        "must not emit the bare `?.chain.toString().count, 0` pattern. Rendered:\n{rendered}"
    );
}
