//! Go assertion rendering.

use crate::e2e::fixture::Assertion;

use streaming_fields::render_streaming_field_assertion;
use synthetic_fields::render_synthetic_field_assertion;
use target::resolve_assertion_target;
use wildcard_assertions::{emit_non_empty_precondition, render_wildcard_or_unavailable_field};

/// Call-invariant state shared by every assertion rendered for one fixture's test
/// function: the same values are passed to every `render_assertion` call in a given
/// assertion loop, in contrast to `assertion`, which varies per call. Immutable by
/// construction -- every field is a borrow or a `Copy` value.
pub(super) struct AssertionRenderContext<'a> {
    pub(super) effective_result_var: &'a str,
    pub(super) import_alias: &'a str,
    pub(super) field_resolver: &'a crate::e2e::field_access::FieldResolver,
    pub(super) optional_locals: &'a std::collections::HashMap<String, String>,
    pub(super) numeric_scalar_fields: &'a std::collections::HashSet<&'a str>,
    /// Fields carrying a `not_empty` assertion elsewhere in this fixture -- proof that a
    /// nil/empty value on the field already fails the test via `render_not_empty`, so a
    /// guarded count assertion on the same field does not need its own failing `else`.
    pub(super) presence_checked_fields: &'a std::collections::HashSet<&'a str>,
    pub(super) result_is_simple: bool,
    pub(super) result_is_array: bool,
    pub(super) is_streaming: bool,
    pub(super) streaming_item_type: Option<&'a str>,
}

/// Render one fixture assertion as Go test-body statements into `out`.
///
/// Dispatch order is load-bearing, not incidental:
/// 1. Synthetic chunk/embedding fields never resolve through `FieldResolver`.
/// 2. Streaming virtual fields likewise resolve through the streaming accessor, not
///    `FieldResolver`, and only apply while a fixture is actually streaming.
/// 3. The wildcard/availability router must run BEFORE ordinary field resolution: a
///    bracket-wildcard path (`links[].link_type`) that reached `resolve_assertion_target`
///    first would lower to its index-0 element and silently assert on one element only.
/// 4. Only once all three have declined does `resolve_assertion_target` build the
///    accessor expression an ordinary (non-synthetic, non-streaming, non-wildcard)
///    assertion renders against.
pub(super) fn render_assertion(out: &mut String, context: &AssertionRenderContext<'_>, assertion: &Assertion) {
    if render_synthetic_field_assertion(out, assertion, context) {
        return;
    }
    if render_streaming_field_assertion(out, assertion, context) {
        return;
    }
    if render_wildcard_or_unavailable_field(out, assertion, context) {
        return;
    }

    let target = resolve_assertion_target(assertion, context);

    // Buffered rather than written straight to `out`: the array-index-0 precondition
    // below must appear BEFORE this assertion's own lines, but whether it is needed
    // (and what array it guards) is only known once the target has produced
    // `array_guard` -- so the assertion's own output must be held until that decision
    // is made, not interleaved with it.
    let mut assertion_buf = String::new();
    dispatch_assertion_type(&mut assertion_buf, context, assertion, &target);

    match &target.array_guard {
        Some(arr) if !assertion_buf.is_empty() => {
            emit_non_empty_precondition(out, arr);
            out.push_str(&assertion_buf);
        }
        _ => out.push_str(&assertion_buf),
    }
}

/// Family dispatcher: route one assertion's rendering to its type-specific helper.
fn dispatch_assertion_type(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &target::ResolvedAssertionTarget,
) {
    match assertion.assertion_type.as_str() {
        "equals" => string_matches::render_equals(out_ref, assertion, target),
        "contains" => string_matches::render_contains(out_ref, context, assertion, target),
        "contains_all" => string_matches::render_contains_all(out_ref, context, assertion, target),
        "not_contains" => string_matches::render_not_contains(out_ref, context, assertion, target),
        "not_empty" => presence::render_not_empty(out_ref, context, assertion, target),
        "is_empty" => presence::render_is_empty(out_ref, context, assertion, target),
        "contains_any" => string_matches::render_contains_any(out_ref, context, assertion, target),
        "greater_than" => comparisons::render_greater_than(out_ref, assertion, target),
        "less_than" => comparisons::render_less_than(out_ref, assertion, target),
        "greater_than_or_equal" => comparisons::render_greater_than_or_equal(out_ref, assertion, target),
        "less_than_or_equal" => comparisons::render_less_than_or_equal(out_ref, assertion, target),
        "starts_with" => string_matches::render_starts_with(out_ref, assertion, target),
        "count_min" => size_ops::render_count_min(out_ref, context, assertion, target),
        "count_equals" => size_ops::render_count_equals(out_ref, context, assertion, target),
        "is_true" => presence::render_is_true(out_ref, target),
        "is_false" => presence::render_is_false(out_ref, target),
        "method_result" => method_result::render_method_result(out_ref, context, assertion),
        "min_length" => size_ops::render_min_length(out_ref, context, assertion, target),
        "max_length" => size_ops::render_max_length(out_ref, context, assertion, target),
        "ends_with" => string_matches::render_ends_with(out_ref, assertion, target),
        "matches_regex" => string_matches::render_matches_regex(out_ref, assertion, target),
        "not_error" | "error" => {}
        other => {
            panic!("Go e2e generator: unsupported assertion type: {other}");
        }
    }
}

#[path = "assertions/synthetic_fields.rs"]
mod synthetic_fields;

#[path = "assertions/streaming_fields.rs"]
mod streaming_fields;

#[path = "assertions/target.rs"]
mod target;

#[path = "assertions/wildcard_assertions.rs"]
mod wildcard_assertions;

#[path = "assertions/comparisons.rs"]
mod comparisons;

#[path = "assertions/size_ops.rs"]
mod size_ops;

#[path = "assertions/presence.rs"]
mod presence;

#[path = "assertions/string_matches.rs"]
mod string_matches;

#[path = "assertions/method_result.rs"]
mod method_result;

#[cfg(test)]
#[path = "assertions/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "assertions/streaming_skip_marker_tests.rs"]
mod streaming_skip_marker_tests;
