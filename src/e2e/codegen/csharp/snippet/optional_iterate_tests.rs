//! Unit-level regression coverage for `dereference_optional_iterate_collections`.
//!
//! Split from `render_csharp_with_optionals` (`src/e2e/field_access/optional_renderers.rs`) after
//! that fix proved too broad: it added the null-forgiving `!` to every optional LEAF regardless of
//! context, which broke `test_accessor_csharp`/`test_accessor_csharp_with_optionals`
//! (`src/e2e/field_access/tests.rs`) -- reading an optional scalar leaf bare (an assertion, a
//! `Console.WriteLine`) is not a dereference and needs no `!`, and every other backend's accessor
//! contract agrees (Rust unwraps the optional PARENT, never the leaf). The real defect is narrower:
//! an optional COLLECTION consumed directly by a `foreach` IS a dereference (`GetEnumerator()`),
//! so the fix belongs at the one place that builds a `foreach` source, not in the shared accessor
//! every context reads a field path through.

use super::*;
use crate::e2e::codegen::presentation::PresentationOperation;

fn iterate_op(expression: &str, optional: bool) -> PresentationOperation {
    PresentationOperation {
        kind: "iterate",
        expression: expression.to_string(),
        item: "item".to_string(),
        fields: Vec::new(),
        optional,
        display: false,
        destructure_source: String::new(),
        destructure_item: String::new(),
        shown_optional: false,
        field_optionals: Vec::new(),
        field_displays: Vec::new(),
        guard_binding: String::new(),
        guard_source: String::new(),
        guard_condition: String::new(),
    }
}

fn show_op(expression: &str, optional: bool) -> PresentationOperation {
    PresentationOperation {
        kind: "show",
        expression: expression.to_string(),
        item: String::new(),
        fields: Vec::new(),
        optional,
        display: false,
        destructure_source: String::new(),
        destructure_item: String::new(),
        shown_optional: false,
        field_optionals: Vec::new(),
        field_displays: Vec::new(),
        guard_binding: String::new(),
        guard_source: String::new(),
        guard_condition: String::new(),
    }
}

/// The real defect: an `iterate` operation whose own collection is optional must carry `!` on its
/// `expression`, since that expression becomes the `foreach` source (`GetEnumerator()` on a
/// possibly-null value is exactly the CS8602 shape).
#[test]
fn an_optional_iterate_collection_gets_the_null_forgiving_operator_on_its_expression() {
    let operations = vec![iterate_op("result.Keywords", true)];

    let out = dereference_optional_iterate_collections(operations);

    assert_eq!(out[0].expression, "result.Keywords!");
}

/// Negative control: a non-optional iterate collection must not gain a `!`, or every non-nullable
/// `foreach` source would carry a redundant (and in strict projects, warned-on) operator.
#[test]
fn a_non_optional_iterate_collection_is_left_alone() {
    let operations = vec![iterate_op("result.Keywords", false)];

    let out = dereference_optional_iterate_collections(operations);

    assert_eq!(out[0].expression, "result.Keywords");
}

/// A `show` operation is never consumed by a `foreach`, so its `expression` must be left alone
/// even when `optional` is set -- the `!` belongs only where a collection is about to be
/// enumerated, not on every optional value this presentation layer touches.
#[test]
fn a_show_operations_expression_is_never_touched_regardless_of_optional() {
    let operations = vec![show_op("result.Title", true)];

    let out = dereference_optional_iterate_collections(operations);

    assert_eq!(out[0].expression, "result.Title");
}

/// Idempotency guard: an expression that already ends in `!` (e.g. because a future accessor
/// change legitimately produces one) must not gain a second one.
#[test]
fn an_expression_already_ending_in_null_forgiving_is_not_doubled_up() {
    let operations = vec![iterate_op("result.Keywords!", true)];

    let out = dereference_optional_iterate_collections(operations);

    assert_eq!(out[0].expression, "result.Keywords!");
}
