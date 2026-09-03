//! Presence/emptiness assertion families (`not_empty`, `is_empty`, `is_true`, `is_false`)
//! for the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::AssertionRenderContext;
use super::target::ResolvedAssertionTarget;

pub(super) fn render_not_empty(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let numeric_scalar_fields = context.numeric_scalar_fields;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_slice = target.field_is_slice;

    let resolved_field = assertion.field.as_deref().unwrap_or("");
    let field_is_array = {
        let rn = field_resolver.resolve(resolved_field);
        field_resolver.is_array(rn)
    };
    // `len()` only compiles against a sized Go type (string, slice, array, map,
    // channel). A field that some *other* assertion in this fixture compares
    // numerically (`equals`/`greater_than[_or_equal]`/`less_than[_or_equal]`
    // against a JSON number) is proven to be a scalar number, not a sized type —
    // `not_empty` cannot call `len()` on it without failing to build. A required
    // numeric scalar always carries a value in Go (there is no zero-length state
    // to detect), so the check degrades to a no-op, matching how `not_empty`
    // already treats "no meaningful check applies" for e.g. `not_error`.
    let is_numeric_scalar = !field_is_pointer && !field_is_array && numeric_scalar_fields.contains(resolved_field);
    if field_is_pointer && !field_is_array {
        let _ = writeln!(out_ref, "\tif {field_expr} == nil {{");
    } else if field_is_nullable && field_is_slice {
        let _ = writeln!(out_ref, "\tif {field_expr} == nil || len({field_expr}) == 0 {{");
    } else if field_is_pointer {
        // `field_is_pointer && field_is_array` falls through the first branch above (which
        // only fires on `!field_is_array`) -- a pointer to an array-like value still needs
        // the `*` this branch supplies. ~keep
        let _ = writeln!(out_ref, "\tif {field_expr} == nil || len(*{field_expr}) == 0 {{");
    } else if field_is_nullable {
        // `field_is_nullable` alone (`is_optional` without `field_is_pointer`) means the
        // resolver marked this field optional but did NOT prove it renders as a Go pointer
        // -- a nilable slice/map/interface, or an unresolved path the resolver refused to
        // guess pointer for (see `assertion_field_shape.rs`'s `unwrap_or(false)`).
        // Dereferencing here regardless of that fact emitted `len(*result.Field)` against a
        // plain, non-pointer nilable type -- a Go compile error, the inverse of the
        // `SheetCount` defect. ~keep
        let _ = writeln!(out_ref, "\tif {field_expr} == nil || len({field_expr}) == 0 {{");
    } else if is_numeric_scalar {
        return;
    } else {
        let _ = writeln!(out_ref, "\tif len({field_expr}) == 0 {{");
    }
    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected non-empty value\")");
    let _ = writeln!(out_ref, "\t}}");
}

pub(super) fn render_is_empty(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let result_is_simple = context.result_is_simple;
    let result_is_array = context.result_is_array;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_slice = target.field_is_slice;

    let field_is_array = {
        let rf = assertion.field.as_deref().unwrap_or("");
        let rn = field_resolver.resolve(rf);
        field_resolver.is_array(rn)
    };
    let simple_scalar_result =
        result_is_simple && !result_is_array && assertion.field.as_ref().is_none_or(|f| f.is_empty());
    if simple_scalar_result || field_is_pointer && !field_is_array {
        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
    } else if field_is_nullable && field_is_slice {
        let _ = writeln!(out_ref, "\tif {field_expr} != nil && len({field_expr}) != 0 {{");
    } else if field_is_pointer {
        let _ = writeln!(out_ref, "\tif {field_expr} != nil && len(*{field_expr}) != 0 {{");
    } else if field_is_nullable {
        // See `render_not_empty`'s identical branch: `field_is_nullable` without
        // `field_is_pointer` must never dereference. ~keep
        let _ = writeln!(out_ref, "\tif {field_expr} != nil && len({field_expr}) != 0 {{");
    } else {
        let _ = writeln!(out_ref, "\tif len({field_expr}) != 0 {{");
    }
    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected empty value, got %v\", {field_expr})");
    let _ = writeln!(out_ref, "\t}}");
}

pub(super) fn render_is_true(out_ref: &mut String, target: &ResolvedAssertionTarget) {
    let field_expr = &target.field_expr;
    let deref_field_expr = &target.deref_field_expr;
    let is_optional = target.is_optional;
    let field_is_pointer = target.field_is_pointer;

    if is_optional {
        // `*T`/`[]T`: "is_true" means "present" -- dereferencing to compare against a
        // bool only type-checks when T is bool, and for a struct field (e.g.
        // `Option<DataNode>`) it does not compile at all. `assert.NotNil` is the
        // interpretation that holds for any T, matching the Rust backend's
        // `.is_some()` convention for the same assertion type. ~keep
        let _ = writeln!(out_ref, "\tassert.NotNil(t, {field_expr}, \"expected true (non-nil)\")");
    } else if field_is_pointer {
        let _ = writeln!(out_ref, "\tassert.True(t, {deref_field_expr}, \"expected true\")");
    } else {
        let _ = writeln!(out_ref, "\tassert.True(t, {field_expr}, \"expected true\")");
    }
}

pub(super) fn render_is_false(out_ref: &mut String, target: &ResolvedAssertionTarget) {
    let field_expr = &target.field_expr;
    let deref_field_expr = &target.deref_field_expr;
    let is_optional = target.is_optional;
    let field_is_pointer = target.field_is_pointer;

    if is_optional {
        let _ = writeln!(out_ref, "\tassert.Nil(t, {field_expr}, \"expected false (nil)\")");
    } else if field_is_pointer {
        let _ = writeln!(out_ref, "\tassert.False(t, {deref_field_expr}, \"expected false\")");
    } else {
        let _ = writeln!(out_ref, "\tassert.False(t, {field_expr}, \"expected false\")");
    }
}
