//! A streaming Swift fixture asserts a counter below `usage` (`usage.total_tokens`) against the
//! LAST chunk of the collected `chunks` array. `Usage` is optional on a chunk, so every step of
//! the read chains with `?.`, and the comparison takes a numeric value.
//!
//! A separate file: `swift/assertions.rs` and `swift/tests.rs` are near the 1,000-line cap.

use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn render(assertion_type: &str, field: &str, value: serde_json::Value, is_streaming: bool) -> String {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &empty_resolver(),
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

/// The defect: the numeric value fell to the value-skip line, so the streaming usage assertion was
/// a comment and the test checked nothing.
#[test]
fn a_numeric_usage_counter_is_asserted_against_the_last_chunk() {
    let out = render("equals", "usage.total_tokens", serde_json::json!(18), true);
    assert_eq!(out, "        XCTAssertEqual(chunks.last?.usage?.totalTokens, 18)\n");
}

#[test]
fn an_ordered_comparison_unwraps_the_optional_counter_first() {
    for (assertion_type, assertion) in [
        ("greater_than", "XCTAssertGreaterThan"),
        ("greater_than_or_equal", "XCTAssertGreaterThanOrEqual"),
        ("less_than", "XCTAssertLessThan"),
        ("less_than_or_equal", "XCTAssertLessThanOrEqual"),
    ] {
        let out = render(assertion_type, "usage.total_tokens", serde_json::json!(5), true);
        assert_eq!(
            out,
            format!("        {assertion}(try XCTUnwrap(chunks.last?.usage?.totalTokens), 5)\n"),
            "{assertion_type}"
        );
    }
}

/// `greater_than` on the counter used to compare the number of chunks with the value, because the
/// arm read `chunks.count` whatever the field. A passing test then said nothing about usage.
#[test]
fn greater_than_on_a_counter_does_not_compare_the_chunk_count() {
    let out = render("greater_than", "usage.total_tokens", serde_json::json!(0), true);
    assert!(!out.contains("chunks.count"), "Emitted:\n{out}");
}

#[test]
fn a_string_valued_usage_field_is_compared_as_a_string() {
    let out = render("equals", "usage.model", serde_json::json!("m"), true);
    assert_eq!(out, "        XCTAssertEqual(chunks.last?.usage?.model, \"m\")\n");
}

/// The same numeric `equals` arm serves the collected list: its length is an `Int`.
#[test]
fn a_numeric_equals_on_the_chunk_length_is_asserted_too() {
    let out = render("equals", "chunks.length", serde_json::json!(3), true);
    assert_eq!(out, "        XCTAssertEqual(chunks.count, 3)\n");
}

/// A call that does not stream has no `chunks`: `usage` is a member of the response.
#[test]
fn a_non_streaming_usage_assertion_never_reads_the_chunks() {
    let out = render("equals", "usage.total_tokens", serde_json::json!(18), false);
    assert!(out.contains("result"), "must read the response. Emitted:\n{out}");
    assert!(
        !out.contains("chunks"),
        "a non-streaming usage assertion must not read the chunks. Emitted:\n{out}"
    );
}
