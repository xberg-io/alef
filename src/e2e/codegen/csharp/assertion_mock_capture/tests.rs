use super::*;

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

/// Exact emitted text for `mock.requests.total` -- the required-proof test: not
/// `contains("mock")`, the whole rendered line. ~keep
#[test]
fn renders_the_exact_text_for_a_total_equals_assertion() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!(3));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion));
    assert_eq!(
        out,
        "        Assert.Equal(3, alefMockRequestCount(Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\", \"/__alef/requests/total?prefix=\"));\n"
    );
}

/// Exact emitted text for a bracketed `"<METHOD> <path>"` key -- the second required-proof case.
/// The key is percent-encoded by `build_path_and_query` before it ever reaches this backend, so
/// this also pins that the emitted string literal carries no unescaped space or slash.
#[test]
fn renders_the_exact_text_for_a_method_and_path_key_greater_than_or_equal_assertion() {
    let assertion = mock_assertion(
        r#"mock.requests["POST /v1/chat"]"#,
        "greater_than_or_equal",
        serde_json::json!(2),
    );
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion));
    assert_eq!(
        out,
        "        Assert.True(alefMockRequestCount(Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\", \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 2);\n"
    );
}

#[test]
fn count_min_is_an_alias_for_greater_than_or_equal() {
    let assertion = mock_assertion("mock.requests.total", "count_min", serde_json::json!(1));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion));
    assert!(out.contains(">= 1"), "got: {out}");
}

#[test]
fn count_equals_is_an_alias_for_equals() {
    let assertion = mock_assertion("mock.requests.total", "count_equals", serde_json::json!(4));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion));
    assert!(out.starts_with("        Assert.Equal(4,"), "got: {out}");
}

#[test]
fn not_equals_maps_to_assert_not_equal() {
    let assertion = mock_assertion("mock.requests.total", "not_equals", serde_json::json!(5));
    let mut out = String::new();
    assert!(try_render_mock_capture_assertion(&mut out, &assertion));
    assert!(out.starts_with("        Assert.NotEqual(5,"), "got: {out}");
}

/// A non-`mock.*` field must fall through untouched -- the interception must not fire on every
/// assertion, only ones whose field actually parses against the `mock.*` grammar.
#[test]
fn a_non_mock_field_is_left_untouched() {
    let assertion = mock_assertion("metadata.title", "equals", serde_json::json!("x"));
    let mut out = String::new();
    assert!(!try_render_mock_capture_assertion(&mut out, &assertion));
    assert_eq!(out, "");
}

/// An assertion with no field at all (e.g. a bare `not_error`) must also fall through.
#[test]
fn a_fieldless_assertion_is_left_untouched() {
    let assertion = Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    };
    let mut out = String::new();
    assert!(!try_render_mock_capture_assertion(&mut out, &assertion));
    assert_eq!(out, "");
}

#[test]
#[should_panic(expected = "unsupported assertion type 'contains' on mock.* field")]
fn an_unsupported_assertion_type_panics() {
    let assertion = mock_assertion("mock.requests.total", "contains", serde_json::json!(1));
    let mut out = String::new();
    let _ = try_render_mock_capture_assertion(&mut out, &assertion);
}

#[test]
#[should_panic(expected = "requires a numeric value in the fixture")]
fn a_non_numeric_value_panics() {
    let assertion = mock_assertion("mock.requests.total", "equals", serde_json::json!("three"));
    let mut out = String::new();
    let _ = try_render_mock_capture_assertion(&mut out, &assertion);
}

mod append_helper_if_referenced_tests {
    use super::append_helper_if_referenced;

    #[test]
    fn appends_the_helper_when_the_body_calls_it() {
        let mut body = "        Assert.Equal(1, alefMockRequestCount(base, \"path\"));\n".to_string();
        append_helper_if_referenced(&mut body);
        assert!(
            body.contains("private static int alefMockRequestCount(string baseUrl, string pathAndQuery)"),
            "got: {body}"
        );
    }

    #[test]
    fn leaves_the_body_untouched_when_the_helper_is_never_called() {
        let mut body = "        Assert.True(result != null);\n".to_string();
        let original = body.clone();
        append_helper_if_referenced(&mut body);
        assert_eq!(
            body, original,
            "a body with no call to the helper must not gain a definition"
        );
    }
}
