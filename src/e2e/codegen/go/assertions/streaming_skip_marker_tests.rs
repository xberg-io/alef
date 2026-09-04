//! Regression coverage for Go streaming-field skip-marker rendering: an accessor that
//! resolves must render a real assertion, and one that doesn't (missing accessor,
//! unrenderable value, unsupported assertion type) must render a counted marker rather
//! than vanish or panic.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::*;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;
use std::collections::{HashMap, HashSet};

/// Run the shared field funnel over a rendered body and return its verdicts, so a test can
/// assert what the gate DECIDED rather than only what the text says. ~keep
fn field_verdicts(body: &str, language: &str) -> Vec<crate::e2e::codegen::SkipVerdict> {
    let _ = crate::e2e::codegen::take_skip_records();
    crate::e2e::codegen::fail_on_unavailable_field_markers(body, language, "streaming_smoke", &[]);
    crate::e2e::codegen::take_skip_records()
        .into_iter()
        .map(|record| record.verdict)
        .collect()
}

fn render_streaming(assertion: &Assertion) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let mut out = String::new();
    let context = AssertionRenderContext {
        effective_result_var: "result",
        import_alias: "pkg",
        field_resolver: &resolver,
        optional_locals: &HashMap::new(),
        numeric_scalar_fields: &HashSet::new(),
        presence_checked_fields: &HashSet::new(),
        result_is_simple: false,
        result_is_array: false,
        is_streaming: true,
        streaming_item_type: None,
    };
    render_assertion(&mut out, &context, assertion);
    out
}

fn streaming_assertion(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
        ..Default::default()
    }
}

/// Non-vacuity control for the two tests below: the same harness on a field whose streaming
/// accessor DOES resolve must produce a real assertion, or "no marker" would prove nothing
/// about markers and everything about the harness. ~keep
#[test]
fn the_streaming_harness_renders_a_real_assertion_when_the_accessor_resolves() {
    let out = render_streaming(&streaming_assertion("count_min", "chunks", Some(serde_json::json!(2))));
    assert!(out.contains("assert.GreaterOrEqual(t, len(chunks), 2"), "got: {out}");
    assert_eq!(
        FieldSkip::extract(&out),
        None,
        "a rendered assertion must carry no skip: {out}"
    );
}

/// `stream.has_page_event` has no accessor without a resolved item type, and this call site
/// passes `None`. The pre-fix renderer emitted NOTHING and returned, so the assertion vanished
/// with no line for the gate to see. Asserting the verdict — not just the text — is the point.
#[test]
fn a_streaming_field_with_no_accessor_emits_a_counted_marker() {
    let out = render_streaming(&streaming_assertion("is_true", "stream.has_page_event", None));
    assert!(!out.is_empty(), "the assertion must not vanish");
    assert_eq!(
        FieldSkip::extract_classified(out.trim_end()),
        Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
        "got: {out}"
    );
    assert_eq!(
        field_verdicts(&out, "go"),
        vec![crate::e2e::codegen::SkipVerdict::AwaitingGeneratorSupport],
        "a missing generator feature must be counted, never fatal: {out}"
    );
}

/// A value the renderer cannot narrow used to leave the arm silent. It now renders a wording
/// the assertion-type funnel recognises.
#[test]
fn a_streaming_assertion_with_an_unrenderable_value_emits_a_counted_marker() {
    let out = render_streaming(&streaming_assertion(
        "count_min",
        "chunks",
        Some(serde_json::json!("not a number")),
    ));
    assert_eq!(
        crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip::extract_classified(out.trim_end()),
        Some((
            "count_min",
            crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip::StreamingAssertionValueNotRenderable
        )),
        "got: {out}"
    );
}

/// An assertion type the streaming renderer does not implement used to render
/// `// streaming field '<f>': assertion type '<t>' not rendered`, which matched no registered
/// shape at all.
#[test]
fn an_unrenderable_streaming_assertion_type_emits_a_counted_marker() {
    let out = render_streaming(&streaming_assertion("matches_regex", "chunks", None));
    assert_eq!(
        crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip::extract_classified(out.trim_end()),
        Some((
            "matches_regex",
            crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip::StreamingAssertionTypeNotSupported
        )),
        "got: {out}"
    );
}
