//! Size assertion families (`count_min`, `count_equals`, `min_length`, `max_length`) for
//! the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;

use super::super::assertion_render_helpers::{
    CountAssertionShape, LengthAssertionShape, render_count_assertion, render_length_assertion,
};
use super::AssertionRenderContext;
use super::target::ResolvedAssertionTarget;

/// Whether some other assertion in this fixture already asserts presence (`not_empty`)
/// on the exact field `assertion` targets -- see `presence_checked_fields` on
/// `AssertionRenderContext`.
fn has_sibling_presence_check(context: &AssertionRenderContext<'_>, assertion: &Assertion) -> bool {
    assertion
        .field
        .as_deref()
        .is_some_and(|f| context.presence_checked_fields.contains(f))
}

pub(super) fn render_count_min(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_expr = target.field_expr.clone();
    let field_is_slice = target.field_is_slice;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();
    let has_sibling = has_sibling_presence_check(context, assertion);

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_count_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr,
            CountAssertionShape {
                is_slice: field_is_slice,
                exact: false,
                has_sibling_presence_check: has_sibling,
            },
        );
    }
}

pub(super) fn render_count_equals(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_expr = target.field_expr.clone();
    let field_is_slice = target.field_is_slice;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();
    let has_sibling = has_sibling_presence_check(context, assertion);

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_count_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr,
            CountAssertionShape {
                is_slice: field_is_slice,
                exact: true,
                has_sibling_presence_check: has_sibling,
            },
        );
    }
}

pub(super) fn render_min_length(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();
    let has_sibling = has_sibling_presence_check(context, assertion);

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_length_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr,
            LengthAssertionShape {
                is_pointer: field_is_pointer,
                minimum: true,
                has_sibling_presence_check: has_sibling,
            },
        );
    }
}

pub(super) fn render_max_length(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();
    let has_sibling = has_sibling_presence_check(context, assertion);

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_length_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr,
            LengthAssertionShape {
                is_pointer: field_is_pointer,
                minimum: false,
                has_sibling_presence_check: has_sibling,
            },
        );
    }
}
