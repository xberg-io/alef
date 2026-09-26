use super::*;

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

/// Exact emitted text for `mock.requests.total` in a synchronous (non-async) test -- the
/// required-proof test named in the task: not `contains("mock")`, the whole rendered line. ~keep
#[test]
fn renders_the_exact_text_for_a_sync_total_equals_assertion() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(3));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion, false));
    assert_eq!(
        out,
        "    assert _alef_mock_request_count(os.environ[\"MOCK_SERVER_URL\"], \"/__alef/requests/total?prefix=\") == 3, \"expected mock.requests.total == 3\"\n"
    );
}

/// Exact emitted text for a bracketed `"<METHOD> <path>"` key in an async test -- the second
/// required-proof case. The field name is embedded in the assertion message and carries literal
/// `"` characters, so this also pins that `escape_python` actually ran. ~keep
#[test]
fn renders_the_exact_text_for_an_async_method_and_path_key_greater_than_or_equal_assertion() {
    let assertion = mock_assertion(
        r#"mock.requests["POST /v1/chat"]"#,
        "greater_than_or_equal",
        serde_json::json!(2),
    );
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion, true));
    assert_eq!(
        out,
        "    assert await asyncio.to_thread(_alef_mock_request_count, os.environ[\"MOCK_SERVER_URL\"], \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 2, \"expected mock.requests[\\\"POST /v1/chat\\\"] >= 2\"\n"
    );
}

#[test]
fn count_min_is_an_alias_for_greater_than_or_equal() {
    let assertion = mock_assertion("mock.requests.total", "count_min", serde_json::json!(1));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion, false));
    assert!(out.contains(">= 1"), "got: {out}");
}

#[test]
fn count_equals_is_an_alias_for_equals() {
    let assertion = mock_assertion("mock.requests.total", "count_equals", serde_json::json!(4));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion, false));
    assert!(out.contains("== 4"), "got: {out}");
}

/// A non-`mock.*` field must fall through untouched.
#[test]
fn a_non_mock_field_is_left_untouched() {
    let assertion = mock_assertion("metadata.title", "equals", serde_json::json!("x"));
    let mut out = String::new();
    assert!(!try_render_mock_capture_assertion(&mut out, &assertion, false));
    assert_eq!(out, "");
}

#[test]
fn a_fieldless_assertion_is_left_untouched() {
    let assertion = Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    };
    let mut out = String::new();
    assert!(!try_render_mock_capture_assertion(&mut out, &assertion, false));
    assert_eq!(out, "");
}

#[test]
#[should_panic(expected = "unsupported assertion type 'contains' on mock.* field")]
fn an_unsupported_assertion_type_panics() {
    let assertion = mock_assertion("mock.requests.total", "contains", serde_json::json!(1));
    let mut out = String::new();
    let _ = try_render_mock_capture_assertion(&mut out, &assertion, false);
}

#[test]
#[should_panic(expected = "requires a numeric value in the fixture")]
fn a_non_numeric_value_panics() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!("three"));
    let mut out = String::new();
    let _ = try_render_mock_capture_assertion(&mut out, &assertion, false);
}
