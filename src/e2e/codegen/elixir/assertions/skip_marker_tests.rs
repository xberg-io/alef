//! Regression coverage for Elixir's skip-marker rendering on unavailable/unsupported
//! streaming assertions.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::render_assertion;
use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::{SkipVerdict, fail_on_unavailable_field_markers, take_skip_records};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn render_streaming(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
        ..Assertion::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        "Sample",
        &HashSet::new(),
        &HashMap::new(),
        false,
        true,
        false,
        false,
        false,
    );
    out
}

fn field_verdicts(body: &str) -> Vec<SkipVerdict> {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(body, "elixir", "stream_smoke", &[]);
    take_skip_records().into_iter().map(|record| record.verdict).collect()
}

/// Non-vacuity control: the same harness on a resolvable streaming field must render a real
/// `assert`, or the marker assertions below would be facts about the harness. ~keep
#[test]
fn the_streaming_harness_renders_a_real_assertion_when_the_accessor_resolves() {
    let out = render_streaming("count_min", "chunks", Some(serde_json::json!(2)));
    assert!(out.contains("assert length("), "got: {out}");
    assert!(
        field_verdicts(&out).is_empty(),
        "a live assertion records no skip: {out}"
    );
}

/// The marker must use elixir's `#` comment opener, not `//`: a `//`-prefixed line is a syntax
/// error in an `.exs` file, so getting this wrong trades a silent drop for a broken build. ~keep
#[test]
fn a_streaming_field_with_no_accessor_emits_a_counted_hash_comment() {
    let out = render_streaming("is_true", "stream.has_page_event", None);
    assert!(out.trim_start().starts_with('#'), "elixir comments start with #: {out}");
    assert_eq!(
        FieldSkip::extract_classified(out.trim_end()),
        Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
        "got: {out}"
    );
    assert_eq!(field_verdicts(&out), vec![SkipVerdict::AwaitingGeneratorSupport]);
}

/// This arm used to `panic!`, aborting the whole run rather than one backend — and a generator
/// gap must never fail a consumer's build at all. It is counted instead. ~keep
#[test]
fn an_unrenderable_streaming_assertion_type_is_counted_rather_than_panicking() {
    let out = render_streaming("matches_regex", "chunks", None);
    assert!(out.trim_start().starts_with('#'), "got: {out}");
    assert_eq!(
        AssertionTypeSkip::extract_classified(out.trim_end()),
        Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
        "got: {out}"
    );
}
