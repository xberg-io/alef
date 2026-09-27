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

/// `"wasm"` shares `"node"`'s capture-table entry (same template, same helper name -- see
/// `mock_assertions::snippets::MOCK_CAPTURE_TABLE`'s `"wasm"` row) because WASM's e2e tests render
/// through this exact function under the identical vitest-on-Node.js harness. `render` must
/// therefore emit byte-identical text for both languages. ~keep
#[test]
fn wasm_shares_nodes_capture_and_renders_identically() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(1));

    let mut node_out = String::new();
    assert!(render(&mut node_out, &assertion, "mock.requests.total", "node"));

    let mut wasm_out = String::new();
    assert!(render(&mut wasm_out, &assertion, "mock.requests.total", "wasm"));

    assert_eq!(
        node_out, wasm_out,
        "wasm and node must render the identical mock-capture call"
    );
    assert_eq!(
        wasm_out,
        "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/total?prefix=\")).toBe(1);\n"
    );
}

/// A language with no capture-table entry at all -- `render` must decline (not panic, not write
/// anything) so the caller falls through to that language's own existing field handling.
/// `"homebrew"` is structurally exempt from ever gaining a row (see
/// `mock_assertions::tests::UNWIRED_LANGUAGE`'s doc), so this stays true rather than needing to
/// move again the next time a language is wired.
#[test]
fn an_unwired_language_has_no_capture_and_is_left_untouched() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(1));
    let mut out = String::new();
    assert!(!render(&mut out, &assertion, "mock.requests.total", "homebrew"));
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
