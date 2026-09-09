use super::render_assertion;
use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::{SkipVerdict, fail_on_unavailable_field_markers, take_skip_records};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn render(assertion: &Assertion, is_streaming: bool) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        is_streaming,
        false,
        "Sample",
    );
    out
}

fn assertion_on(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
        ..Assertion::default()
    }
}

/// Run the shared field funnel over a rendered body and return its verdicts, so a test can
/// assert what the gate DECIDED rather than only what the text says. ~keep
fn field_verdicts(body: &str) -> Vec<SkipVerdict> {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(body, "swift", "swift_smoke", &[]);
    take_skip_records().into_iter().map(|record| record.verdict).collect()
}

/// Non-vacuity control for every test below: the same harness on an ordinary field must render
/// a real `XCTAssert`, or "no marker" / "one marker" would be a fact about the harness rather
/// than about markers. ~keep
#[test]
fn the_harness_renders_a_real_assertion_for_an_ordinary_field() {
    let out = render(
        &assertion_on("equals", "title", Some(serde_json::json!("hello"))),
        false,
    );
    assert!(out.contains("XCTAssert"), "got: {out}");
    assert!(
        field_verdicts(&out).is_empty(),
        "a live assertion records no skip: {out}"
    );
}

/// The collision the consumer hit: `headings.length` RESOLVES (the guard runs only after
/// `is_valid_for_result` accepted it) but swift-bridge JSON-bridges the leaf to a `RustString`
/// with no `.count`, so the backend honestly refuses it. It used to refuse in
/// `NotAvailableOnResultType`'s words — an `AuthoringGap`, and therefore FATAL — so the
/// backend called it an ABI limit and the strict gate called it a broken field path, about the
/// same field, at the same moment. Asserting the VERDICT is what stops the two disagreeing
/// again: reclassifying this variant fails here rather than silently failing consumers. ~keep
#[test]
fn a_json_bridged_count_is_a_limitation_never_an_unacknowledged_gap() {
    let out = render(
        &assertion_on("count_equals", "headings.length", Some(serde_json::json!(3))),
        false,
    );
    assert_eq!(
        FieldSkip::extract_classified(out.trim_end()),
        Some(("headings.length", FieldSkip::CountOnJsonBridgedLeafInSwift)),
        "got: {out}"
    );
    assert_eq!(
        field_verdicts(&out),
        vec![SkipVerdict::Limitation],
        "a resolvable field refused for an ABI reason must never be an unacknowledged gap: {out}"
    );
}

/// `stream.has_page_event` has no accessor without a resolved item type, and swift's call site
/// supplies none. The pre-fix renderer emitted NOTHING and returned, so the assertion vanished
/// and the emitted test body could end up with no assertion call at all — permanently green.
#[test]
fn a_streaming_field_with_no_accessor_emits_a_counted_marker() {
    let out = render(&assertion_on("is_true", "stream.has_page_event", None), true);
    assert!(!out.is_empty(), "the assertion must not vanish");
    assert_eq!(
        FieldSkip::extract_classified(out.trim_end()),
        Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
        "got: {out}"
    );
    assert_eq!(
        field_verdicts(&out),
        vec![SkipVerdict::AwaitingGeneratorSupport],
        "alef's own generator debt is counted, never fatal: {out}"
    );
}

/// A streaming assertion type the renderer does not implement used to emit
/// `// streaming field '<f>': assertion type '<t>' not rendered`, which matched no registered
/// shape and carried no `skipped:` prefix — invisible to both funnels and to a grep census.
#[test]
fn an_unrenderable_streaming_assertion_type_emits_a_registered_marker() {
    let out = render(&assertion_on("matches_regex", "chunks", None), true);
    assert_eq!(
        AssertionTypeSkip::extract_classified(out.trim_end()),
        Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
        "got: {out}"
    );
}

/// A value the renderer cannot narrow used to leave the arm silent.
#[test]
fn an_unrenderable_streaming_value_emits_a_registered_marker() {
    let out = render(
        &assertion_on("count_min", "chunks", Some(serde_json::json!("three"))),
        true,
    );
    assert_eq!(
        AssertionTypeSkip::extract_classified(out.trim_end()),
        Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
        "got: {out}"
    );
}
