use super::{apply_vacuous_assertion_fallback, has_executable_assertion};

#[test]
fn has_executable_assertion_is_false_for_comment_only_body() {
    let body = "        // skipped: field 'foo' not available on result type\n";
    assert!(
        !has_executable_assertion(body),
        "comment-only body must not count as asserting"
    );
}

#[test]
fn has_executable_assertion_is_false_for_empty_body() {
    assert!(!has_executable_assertion(""));
    assert!(!has_executable_assertion("   \n  \n"));
}

#[test]
fn has_executable_assertion_is_true_when_a_real_statement_is_present() {
    let body =
        "        // skipped: field 'foo' not available on result type\n        $this->assertTrue($result->ok);\n";
    assert!(
        has_executable_assertion(body),
        "a real assertTrue line must count as asserting"
    );
}

/// Regression test for the void `not_error` defect: before this fix, a `returns_void`
/// fixture whose only assertion was `not_error` fell into the non-streaming branch and
/// emitted `$this->assertNotNull($result);` — but a void call's binding return is always
/// PHP `null` on success, so that assertion FAILED on every successful call, not just an
/// unsuccessful one. Worse than the vacuous body it was meant to replace.
#[test]
fn void_not_error_body_gets_a_real_assertion_that_a_successful_call_can_pass() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, false, false, "result", false, true);
    assert_eq!(
        body,
        "        $this->assertTrue(true, 'expected the call not to throw');\n"
    );
    assert!(
        !body.contains("assertNotNull"),
        "a void call's result is always null; assertNotNull would fail every successful call, got: {body}"
    );
}

/// Regression test for the not_error-only vacuous-test defect: a fixture whose
/// only assertion is `not_error` renders an empty assertions_body. The fallback
/// must emit a real assertion, never `expectNotToPerformAssertions()`.
#[test]
fn not_error_only_body_gets_a_real_assertion_not_a_framework_suppression() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, false, false, "result", false, false);
    assert_eq!(body, "        $this->assertNotNull($result);\n");
    assert!(!body.contains("expectNotToPerformAssertions"));
}

#[test]
fn comment_only_body_also_gets_a_real_assertion() {
    let mut body = "        // skipped: field 'chunks' not available on result type\n".to_string();
    apply_vacuous_assertion_fallback(&mut body, false, false, "result", false, false);
    assert!(
        body.contains("$this->assertNotNull($result);"),
        "a comment-only body must still get a real fallback assertion, got: {body}"
    );
    assert!(!body.contains("expectNotToPerformAssertions"));
}

/// A `not_error` streaming fixture really does check that the drive did not throw, so the
/// line that keeps PHPUnit from filing it as risky is still emitted beside that real check.
#[test]
fn streaming_not_error_body_keeps_the_line_that_marks_the_test_non_risky() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, true, false, "result", true, false);
    assert_eq!(
        body,
        "        $this->assertTrue(is_array($chunks), 'expected drained chunks list');\n"
    );
}

/// `$chunks` is bound to a freshly drained array immediately above, so `is_array($chunks)`
/// cannot fail. Emitting it for a fixture that declared real assertions and had every one of
/// them dropped is what published the example as a permanent green; the body must be left
/// comment-only so the refusal in `render_test_method` can see it. ~keep
#[test]
fn streaming_body_without_not_error_gets_no_guard_that_cannot_fail() {
    let mut body = "        // skipped: field 'x' not available on result type\n".to_string();
    let original = body.clone();
    apply_vacuous_assertion_fallback(&mut body, true, false, "result", false, false);
    assert_eq!(
        body, original,
        "a check that cannot fail must not be injected in place of the dropped assertions"
    );
}

#[test]
fn error_expecting_fixture_is_left_untouched() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, false, true, "result", false, false);
    assert!(
        body.is_empty(),
        "error-expecting fixtures render their own error_test_body, not this fallback"
    );
}

#[test]
fn body_with_a_real_assertion_is_left_untouched() {
    let mut body = "        $this->assertEquals(1, $result->count);\n".to_string();
    let original = body.clone();
    apply_vacuous_assertion_fallback(&mut body, false, false, "result", false, false);
    assert_eq!(
        body, original,
        "a fixture with a real assertion must not get an extra fallback line"
    );
}

/// Regression test for alef task #81: PHP's "skipped: field not available" comment
/// text must survive as the exact marker the shared `fail_on_unavailable_field_markers`
/// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
/// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
/// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
/// `unavailable_field_marker_tests`; this test only pins the marker text PHP emits.
#[test]
fn skip_comment_carries_the_marker_the_strict_mode_matches_on() {
    let mut body = "        // skipped: field 'nonexistent_field' not available on result type\n".to_string();
    // Confirm the comment alone doesn't get treated as a real assertion.
    assert!(!has_executable_assertion(&body));
    apply_vacuous_assertion_fallback(&mut body, false, false, "result", false, false);
    assert!(
        body.contains("field 'nonexistent_field' not available on result type"),
        "got: {body}"
    );
}
