use super::*;

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

/// Exact emitted text for `mock.requests.total` -- the required-proof test named in the task: not
/// `contains("mock")`, the whole rendered line. ~keep
#[test]
fn renders_the_exact_text_for_a_total_equals_assertion() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(3));
    let mut out = String::new();
    assert!(render(&mut out, &assertion, "mock.requests.total", "node"));
    assert_eq!(
        out,
        "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/total?prefix=\")).toBe(3);\n"
    );
}

/// Exact emitted text for a bracketed `"<METHOD> <path>"` key -- the second required-proof case.
#[test]
fn renders_the_exact_text_for_a_method_and_path_key_greater_than_or_equal_assertion() {
    let field = r#"mock.requests["POST /v1/chat"]"#;
    let assertion = mock_assertion(field, "greater_than_or_equal", serde_json::json!(2));
    let mut out = String::new();
    assert!(render(&mut out, &assertion, field, "node"));
    assert_eq!(
        out,
        "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\")).toBeGreaterThanOrEqual(2);\n"
    );
}

#[test]
fn count_min_is_an_alias_for_greater_than_or_equal() {
    let assertion = mock_assertion("mock.requests.total", "count_min", serde_json::json!(1));
    let mut out = String::new();
    assert!(render(&mut out, &assertion, "mock.requests.total", "node"));
    assert!(out.contains("toBeGreaterThanOrEqual(1)"), "got: {out}");
}

#[test]
fn count_equals_is_an_alias_for_equals() {
    let assertion = mock_assertion("mock.requests.total", "count_equals", serde_json::json!(4));
    let mut out = String::new();
    assert!(render(&mut out, &assertion, "mock.requests.total", "node"));
    assert!(out.contains(".toBe(4)"), "got: {out}");
}

/// `"wasm"` has no capture-table entry -- `render` must decline (not panic, not write anything)
/// so the caller falls through to WASM's own existing field handling.
#[test]
fn wasm_has_no_capture_and_is_left_untouched() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(1));
    let mut out = String::new();
    assert!(!render(&mut out, &assertion, "mock.requests.total", "wasm"));
    assert_eq!(out, "");
}

/// A non-`mock.*` field must fall through untouched.
#[test]
fn a_non_mock_field_is_left_untouched() {
    let assertion = mock_assertion("metadata.title", "equals", serde_json::json!("x"));
    let mut out = String::new();
    assert!(!render(&mut out, &assertion, "metadata.title", "node"));
    assert_eq!(out, "");
}

#[test]
#[should_panic(expected = "unsupported assertion type 'contains' on mock.* field")]
fn an_unsupported_assertion_type_panics() {
    let assertion = mock_assertion("mock.requests.total", "contains", serde_json::json!(1));
    let mut out = String::new();
    let _ = render(&mut out, &assertion, "mock.requests.total", "node");
}

#[test]
#[should_panic(expected = "requires a numeric value in the fixture")]
fn a_non_numeric_value_panics() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!("three"));
    let mut out = String::new();
    let _ = render(&mut out, &assertion, "mock.requests.total", "node");
}
