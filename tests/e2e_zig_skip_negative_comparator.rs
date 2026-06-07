//! Tests that Zig e2e codegen correctly skips assertions with negative comparators.
//!
//! When a fixture uses assertions like `greater_than: -1` or `greater_than_or_equal: -1`,
//! these are sentinel "always true" checks for unsigned types (e.g., array length > -1 is
//! always true). Zig's type system disallows `@as(usize, -1)`, so the codegen must skip
//! these assertions entirely rather than emit invalid code.
//!
//! Regression: error-handling fixtures with `min_value: -1` sentinel assertions were
//! generating `try testing.expect(result.object.get("links").?.array.items.len > @as(usize, -1));`
//! which fails to compile with "type 'usize' cannot represent integer value '-1'".

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use serde_json::json;

fn config_toml() -> &'static str {
    r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "kreuzcrawl"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "kreuzcrawl"
result_var = "result"
returns_result = true
args = [{ name = "url", field = "url", type = "string" }]
"#
}

fn render_with_fixture(fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(config_toml()).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "error_handling".to_string(),
        fixtures: vec![fixture],
    }];
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("error_handling_test.zig"))
        .expect("error_handling_test.zig is emitted")
        .content
        .clone()
}

/// A fixture with `greater_than: -1` must not emit the invalid Zig assertion.
/// The assertion is a no-op sentinel (length > -1 is always true for unsigned types).
#[test]
fn greater_than_negative_one_is_skipped() {
    let fixture = Fixture {
        id: "links_greater_than_negative".to_string(),
        category: Some("error_handling".to_string()),
        description: "Links count > -1 (sentinel, always true)".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: json!({
            "url": "http://example.com",
            "config": { "html": "<a href='/'>Home</a>" }
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(json!({
                "object": {
                    "links": [
                        { "text": "Home", "href": "/" }
                    ]
                }
            })),
            stream_chunks: None,
            headers: Default::default(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "greater_than".to_string(),
            field: Some("object.links.length".to_string()),
            value: Some(json!(-1)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render_with_fixture(fixture);

    // The assertion must not appear (it's a no-op sentinel).
    assert!(
        !rendered.contains("@as(usize, -1)"),
        "greater_than: -1 must not emit @as(usize, -1). Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("object.links.length > @as("),
        "greater_than: -1 must not emit any comparator assertion. Rendered:\n{rendered}"
    );
}

/// A fixture with `greater_than_or_equal: -1` must not emit the invalid Zig assertion.
#[test]
fn greater_than_or_equal_negative_one_is_skipped() {
    let fixture = Fixture {
        id: "items_gte_negative".to_string(),
        category: Some("error_handling".to_string()),
        description: "Items count >= -1 (sentinel, always true)".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: json!({
            "url": "http://example.com"
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(json!({
                "array": [1, 2, 3]
            })),
            stream_chunks: None,
            headers: Default::default(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "greater_than_or_equal".to_string(),
            field: Some("array.length".to_string()),
            value: Some(json!(-1)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render_with_fixture(fixture);

    assert!(
        !rendered.contains("@as(usize, -1)"),
        "greater_than_or_equal: -1 must not emit @as(usize, -1). Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("array.length >= @as("),
        "greater_than_or_equal: -1 must not emit any comparator assertion. Rendered:\n{rendered}"
    );
}

/// A fixture with `greater_than: 0` (positive value) must still emit the assertion.
#[test]
fn greater_than_positive_value_is_emitted() {
    let fixture = Fixture {
        id: "links_greater_than_zero".to_string(),
        category: Some("error_handling".to_string()),
        description: "Links count > 0 (valid, not a sentinel)".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: json!({
            "url": "http://example.com"
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(json!({
                "object": {
                    "links": [
                        { "text": "Home", "href": "/" }
                    ]
                }
            })),
            stream_chunks: None,
            headers: Default::default(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "greater_than".to_string(),
            field: Some("object.links.length".to_string()),
            value: Some(json!(0)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render_with_fixture(fixture);

    // The assertion must appear when the value is non-negative.
    assert!(
        rendered.contains("object.links.len > 0"),
        "greater_than: 0 must emit a valid assertion. Rendered:\n{rendered}"
    );
}

/// A fixture with `less_than: -1` (negative value) must still emit the assertion,
/// since `less_than` comparisons require correct handling of negative bounds.
#[test]
fn less_than_negative_value_is_emitted() {
    let fixture = Fixture {
        id: "count_less_than_negative".to_string(),
        category: Some("error_handling".to_string()),
        description: "Count < -1 (edge case, must emit)".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: json!({
            "url": "http://example.com"
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(json!({
                "count": -5
            })),
            stream_chunks: None,
            headers: Default::default(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "less_than".to_string(),
            field: Some("count".to_string()),
            value: Some(json!(-1)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render_with_fixture(fixture);

    // less_than comparisons are meaningful even with negative values (e.g., comparing signed ints).
    assert!(
        rendered.contains("result.len < -1"),
        "less_than: -1 must emit assertion. Rendered:\n{rendered}"
    );
}
