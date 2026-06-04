//! Verifies the Zig e2e codegen does NOT emit `try testing.expect(result != null);`
//! for `not_error` assertions, particularly when the call returns `?T` (Optional)
//! and a sibling `is_empty` assertion expects `result == null`.
//!
//! Regression test: previously `not_error` and `not_empty` shared a branch under
//! `bare_result_is_option`, causing `not_error` to incorrectly emit `!= null` —
//! which contradicts a paired `is_empty` assertion in the same test and fails
//! at runtime.
//!
//! In Zig, `not_error` is covered by `try` propagation (the call would have
//! returned early on error). The correct emission is a comment-only line.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        field: None,
        value: None,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn is_empty_assertion() -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: None,
        value: None,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn config_toml() -> &'static str {
    r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "sample_language_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "sample_language_pack"
result_var = "result"
args = [{ name = "source", field = "source_code", type = "string" }]

[crates.e2e.calls.detect_content]
function = "detect_language_from_content"
module = "sample_language_pack"
result_var = "result"
result_is_simple = true
result_is_option = true
args = [{ name = "content", field = "content", type = "string" }]
"#
}

fn render(fixture: Fixture) -> String {
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

/// A fixture with a single `not_error` assertion on an Optional result must
/// NOT emit `try testing.expect(result != null);`. The call is verified by
/// `try` propagation alone — the result is discarded with `_ = try ...`.
#[test]
fn not_error_alone_does_not_emit_not_null_check() {
    let fixture = Fixture {
        id: "error_detect_content_empty".to_string(),
        category: Some("error_handling".to_string()),
        description: "Detect language from empty content returns null".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("detect_content".to_string()),
        input: serde_json::json!({ "content": "" }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![not_error_assertion()],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render(fixture);

    assert!(
        !rendered.contains("try testing.expect(result != null);"),
        "not_error must NOT emit `expect(result != null)` for Optional result. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("_ = try sample_language_pack.detect_language_from_content("),
        "call must still be emitted and `try`-propagated. Rendered:\n{rendered}"
    );
}

/// A fixture with both `not_error` and `is_empty` on an Optional result must
/// NOT emit contradictory `!= null` and `== null` checks. Only the `is_empty`
/// check should produce an assertion.
#[test]
fn not_error_with_is_empty_does_not_contradict() {
    let fixture = Fixture {
        id: "error_detect_content_empty_pair".to_string(),
        category: Some("error_handling".to_string()),
        description: "not_error + is_empty must not contradict".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("detect_content".to_string()),
        input: serde_json::json!({ "content": "" }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![not_error_assertion(), is_empty_assertion()],
        source: "error_handling.json".to_string(),
        http: None,
    };

    let rendered = render(fixture);

    assert!(
        !rendered.contains("try testing.expect(result != null);"),
        "not_error + is_empty must NOT emit `expect(result != null)`. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("try testing.expect(result == null);"),
        "is_empty must still emit `expect(result == null)`. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("// not_error: covered by try propagation"),
        "not_error must emit a comment-only inert line. Rendered:\n{rendered}"
    );
}
