//! Derivation of the Go accessor expression and shape facts for one assertion's target
//! field.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;

use super::super::assertion_field_shape::resolve_assertion_field_shape;
use super::AssertionRenderContext;

/// Resolved facts about the Go expression an assertion's field accessor renders as,
/// bundled together because they are all produced by the same accessor/shape resolution
/// pass and are consumed together by the assertion-type family dispatchers.
pub(super) struct ResolvedAssertionTarget {
    pub(super) field_expr: String,
    pub(super) deref_field_expr: String,
    pub(super) field_is_pointer: bool,
    pub(super) field_is_nullable: bool,
    pub(super) field_is_slice: bool,
    pub(super) field_is_data_interface: bool,
    pub(super) is_optional: bool,
    pub(super) nil_guard_expr: Option<String>,
    pub(super) nullable_guard_expr: Option<String>,
    pub(super) array_guard: Option<String>,
}

/// Resolve the initial (pre-shape-adjustment) Go accessor expression for `assertion`'s target
/// field: the simple-result variable, an already-unwrapped optional local, or the field
/// resolver's own accessor.
fn initial_field_expr(
    assertion: &Assertion,
    result_is_simple: bool,
    result_var: &str,
    optional_locals: &std::collections::HashMap<String, String>,
    field_resolver: &crate::e2e::field_access::FieldResolver,
) -> String {
    if result_is_simple {
        return result_var.to_string();
    }
    match &assertion.field {
        Some(f) if !f.is_empty() => {
            if let Some(local_var) = optional_locals.get(f.as_str()) {
                local_var.clone()
            } else {
                field_resolver.accessor(f, "go", result_var)
            }
        }
        _ => result_var.to_string(),
    }
}

/// Resolve the Go accessor expression and shape facts for `assertion`'s target field.
///
/// Must run after the synthetic-field, streaming-field, and wildcard/availability checks
/// have all declined to handle the assertion: this is the "otherwise, resolve the field
/// normally" path, and lowers a bracket-wildcard field to its index-0 element if run
/// before the wildcard check runs first.
pub(super) fn resolve_assertion_target(
    assertion: &Assertion,
    context: &AssertionRenderContext<'_>,
) -> ResolvedAssertionTarget {
    let result_is_simple = context.result_is_simple;
    let result_var = context.effective_result_var;
    let optional_locals = context.optional_locals;
    let field_resolver = context.field_resolver;

    let field_expr = initial_field_expr(assertion, result_is_simple, result_var, optional_locals, field_resolver);

    let field_shape = resolve_assertion_field_shape(assertion, field_resolver, optional_locals);
    let is_optional = field_shape.is_optional;
    let receiver_is_pointer = field_shape.is_pointer;
    let receiver_is_nullable = field_shape.is_nullable;
    let field_is_array_for_len = field_shape.is_array_for_len;
    let field_is_data_interface = field_shape.is_data_interface;
    let field_expr = if receiver_is_pointer
        && field_expr.starts_with("len(")
        && field_expr.ends_with(')')
        && !field_is_array_for_len
    {
        let inner = &field_expr[4..field_expr.len() - 1];
        format!("len(*{inner})")
    } else {
        field_expr
    };
    let nil_guard_expr = if receiver_is_pointer && field_expr.starts_with("len(*") {
        Some(field_expr[5..field_expr.len() - 1].to_string())
    } else {
        None
    };
    let expression_is_length = field_expr.starts_with("len(");
    let field_is_pointer = receiver_is_pointer && !expression_is_length;
    let field_is_nullable = receiver_is_nullable && !expression_is_length;
    let nullable_guard_expr = nil_guard_expr
        .clone()
        .or_else(|| field_is_nullable.then(|| field_expr.clone()));

    let field_is_slice = field_shape.is_slice;
    let deref_field_expr = if field_is_pointer && !field_expr.starts_with("len(") && !field_is_slice {
        format!("*{field_expr}")
    } else {
        field_expr.clone()
    };

    let array_guard: Option<String> = if let Some(idx) = field_expr.find("[0]") {
        let mut array_expr = field_expr[..idx].to_string();
        if let Some(stripped) = array_expr.strip_prefix("len(") {
            array_expr = stripped.to_string();
        }
        Some(array_expr)
    } else {
        None
    };

    ResolvedAssertionTarget {
        field_expr,
        deref_field_expr,
        field_is_pointer,
        field_is_nullable,
        field_is_slice,
        field_is_data_interface,
        is_optional,
        nil_guard_expr,
        nullable_guard_expr,
        array_guard,
    }
}
