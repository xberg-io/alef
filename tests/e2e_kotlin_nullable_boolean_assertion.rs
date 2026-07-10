//! Verifies the Kotlin e2e codegen handles nullable boolean assertions correctly.
//! Regression test for a bug where boolean is_true/is_false assertions on
//! nullable Boolean? fields (e.g. reached via safe-call chains like `result.markdown?.citations`)
//! would emit `assertTrue(expr)` where expr is Boolean?, causing a compilation error.
//! The fix uses explicit equality comparison: `assertTrue(expr == true)` and `assertTrue(expr == false)`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin::KotlinE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn make_fixture(id: &str, description: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("markdown".to_string()),
        description: description.to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({
            "request": {}
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions,
        source: "markdown.json".to_string(),
        http: None,
    }
}

fn make_group(fixtures: Vec<Fixture>) -> FixtureGroup {
    FixtureGroup {
        category: "markdown".to_string(),
        fixtures,
    }
}

const TOML: &str = r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample-crate"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample.lib"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields = { markdown = "markdown", citations = "markdown.citations" }
result_fields = ["markdown"]

[crates.e2e.call]
function = "process_content"
module = "dev.sample.lib.SampleLib"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "SampleLib"
function = "processContent"

[[crates.e2e.call.args]]
name = "content"
field = "input.content"
type = "string"
"#;

fn render(fixtures: Vec<Fixture>) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixtures)];
    let files = KotlinE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| {
            f.path.to_string_lossy().contains("markdown_test.kt")
                || f.path.to_string_lossy().contains("MarkdownTest.kt")
        })
        .expect("markdown_test.kt is emitted")
        .content
        .clone()
}

/// When a boolean is_true assertion is on a nullable Boolean? field,
/// the generated code must use explicit equality comparison `assertTrue(expr == true, ...)`
/// instead of `assertTrue(expr, ...)` to handle nullable results. This allows the test
/// to compile when the field is reached via safe-call chains.
#[test]
fn nullable_boolean_is_true_assertion_uses_equality_comparison() {
    let fixtures = vec![make_fixture(
        "markdown_with_citations_true",
        "markdown contains citations",
        vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("markdown.citations".to_string()),
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    )];

    let rendered = render(fixtures);

    assert!(
        rendered.contains("== true"),
        "is_true assertion on nullable boolean field must use explicit equality comparison (== true). Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("assertTrue("),
        "must emit assertTrue statement. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("assertTrue(result.markdown?.citations, "),
        "must NOT emit bare assertTrue(expr) for nullable fields. Rendered:\n{rendered}"
    );
}

/// When a boolean is_false assertion is on a nullable Boolean? field,
/// the generated code must use explicit equality comparison `assertTrue(expr == false, ...)`
/// instead of `assertFalse(expr, ...)` to handle nullable results.
#[test]
fn nullable_boolean_is_false_assertion_uses_equality_comparison() {
    let fixtures = vec![make_fixture(
        "markdown_with_citations_false",
        "markdown does not contain citations",
        vec![Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("markdown.citations".to_string()),
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    )];

    let rendered = render(fixtures);

    assert!(
        rendered.contains("== false"),
        "is_false assertion on nullable boolean field must use explicit equality comparison (== false). Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("assertTrue("),
        "must emit assertTrue statement (for consistency, == false tests are wrapped in assertTrue). Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("assertFalse(result.markdown?.citations, "),
        "must NOT emit bare assertFalse(expr) for nullable fields. Rendered:\n{rendered}"
    );
}
