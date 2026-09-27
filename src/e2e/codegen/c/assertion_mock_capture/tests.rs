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
        "    assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 3 && \"equals assertion failed\");\n"
    );
}

/// Exact emitted text for a bracketed `"<METHOD> <path>"` key -- the second required-proof case.
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
        "    assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 2 && \"expected greater than or equal\");\n"
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
    assert!(out.starts_with("    assert(alef_mock_request_count"), "got: {out}");
    assert!(out.contains("== 4"), "got: {out}");
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

/// `append_helper_if_referenced` must insert the extra socket includes at the offset passed in,
/// not at the end -- the helper is appended after the fixture body, but the `#include`s it needs
/// must precede every use, which for a real generated file means the top of the file. ~keep
#[test]
fn append_helper_if_referenced_inserts_includes_at_the_given_offset_and_appends_the_helper() {
    let mut out = String::from("// includes go here\n// fixture body calling alef_mock_request_count(...)\n");
    let includes_offset = "// includes go here\n".len();
    append_helper_if_referenced(&mut out, includes_offset);
    assert!(
        out.starts_with("// includes go here\n#ifdef _WIN32\n"),
        "expected the injected block right after the marker, got:\n{out}"
    );
    for header in [
        "#include <sys/socket.h>\n",
        "#include <netdb.h>\n",
        "#include <unistd.h>\n",
    ] {
        assert!(out.contains(header), "expected {header:?} injected, got:\n{out}");
    }
    assert!(
        out.contains("static int alef_mock_request_count(const char *base_url, const char *path_and_query)"),
        "expected the helper definition appended, got:\n{out}"
    );
}

/// Negative direction: a file whose fixture body never calls the helper gets no includes and no
/// helper definition -- the always-emit bug `ensure_capture_declared`'s own doc warns about.
#[test]
fn append_helper_if_referenced_is_a_no_op_when_the_helper_is_never_called() {
    let mut out = String::from("// includes go here\n// fixture body with no mock.* assertion\n");
    let original = out.clone();
    append_helper_if_referenced(&mut out, "// includes go here\n".len());
    assert_eq!(out, original, "a file with no mock.* call must be left untouched");
}

/// The Windows guard must PRECEDE the first POSIX include, or the preprocessor reaches
/// `<sys/socket.h>` first and the build dies with a missing-header cascade that names nothing
/// useful -- ordering is the entire value of the guard, so assert the offsets, not just presence
/// (alef issue #468). ~keep
#[test]
fn the_windows_guard_precedes_every_posix_include_and_names_the_issue() {
    let mut out = String::from("// includes go here\n// fixture body calling alef_mock_request_count(...)\n");
    append_helper_if_referenced(&mut out, "// includes go here\n".len());

    let guard = out.find("#ifdef _WIN32").expect("expected a _WIN32 guard, got:\n{out}");
    let error_directive = out.find("#error").expect("expected an #error directive");
    let endif = out.find("#endif").expect("expected the guard to be closed");
    let first_posix_include = out
        .find("#include <sys/socket.h>")
        .expect("expected the socket include");

    assert!(guard < error_directive, "the #error must sit inside the guard");
    assert!(error_directive < endif, "the guard must close after the #error");
    assert!(
        endif < first_posix_include,
        "the guard must close BEFORE the first POSIX include, otherwise the missing-header \
         cascade fires first; got:\n{out}"
    );
    assert!(
        out[error_directive..endif].contains("#468"),
        "the #error must name alef issue #468 so the failure is traceable, got:\n{out}"
    );
}
