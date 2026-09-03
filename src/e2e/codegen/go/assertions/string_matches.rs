//! Value-matching assertion families (`equals`, `contains*`, `starts_with`, `ends_with`,
//! `matches_regex`) for the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::super::assertion_render_helpers::{contains_value_expression, string_value_expression};
use super::super::json_values::json_to_go;
use super::AssertionRenderContext;
use super::target::ResolvedAssertionTarget;

pub(super) fn render_equals(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let deref_field_expr = target.deref_field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_data_interface = target.field_is_data_interface;
    let is_optional = target.is_optional;

    if let Some(expected) = &assertion.value {
        let go_val = json_to_go(expected);
        if expected.is_string() {
            let string_field = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
            let expected_string = if field_is_data_interface {
                format!("jsonString(t, {go_val})")
            } else {
                go_val.clone()
            };
            if field_is_nullable && !field_expr.starts_with("len(") {
                let _ = writeln!(
                    out_ref,
                    "\tif {field_expr} == nil || {string_field} != {expected_string} {{"
                );
            } else {
                let _ = writeln!(out_ref, "\tif {string_field} != {expected_string} {{");
            }
        } else if field_is_pointer && !field_expr.starts_with("len(") {
            let _ = writeln!(out_ref, "\tif {field_expr} == nil || {deref_field_expr} != {go_val} {{");
        } else if is_optional && !field_expr.starts_with("len(") {
            // ~keep Latent, unpatched: `is_optional` here can be true while `field_is_pointer`
            // is false for a nilable slice/map (comparing it to a scalar `go_val` fails to
            // compile) OR for a data-interface field, where `field_expr != go_val` against
            // `interface{}` actually DOES compile for a comparable literal -- so this branch is
            // not provably wrong in general. No downstream fixture reaches it with a non-string
            // value today, and no failing case was found to pin a fix against (tdd-workflow:
            // no red test, no patch). Left as-is pending a concrete repro; do not delete this
            // note or "fix" this blind without one.
            let _ = writeln!(out_ref, "\tif {field_expr} != nil && {field_expr} != {go_val} {{");
        } else {
            let _ = writeln!(out_ref, "\tif {field_expr} != {go_val} {{");
        }
        let _ = writeln!(out_ref, "\t\tt.Errorf(\"equals mismatch: got %v\", {field_expr})");
        let _ = writeln!(out_ref, "\t}}");
    }
}

pub(super) fn render_contains(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let result_is_array = context.result_is_array;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(expected) = &assertion.value {
        let go_val = json_to_go(expected);
        let resolved_field = assertion.field.as_deref().unwrap_or("");
        let resolved_name = field_resolver.resolve(resolved_field);
        let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
        let is_nullable = field_is_nullable;
        let field_for_contains =
            contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
        if is_nullable {
            let _ = writeln!(
                out_ref,
                "\tif {field_expr} == nil || !strings.Contains({field_for_contains}, {go_val}) {{"
            );
            let _ = writeln!(
                out_ref,
                "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
            );
            let _ = writeln!(out_ref, "\t}}");
        } else {
            let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
            let _ = writeln!(
                out_ref,
                "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
            );
            let _ = writeln!(out_ref, "\t}}");
        }
    }
}

pub(super) fn render_contains_all(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let result_is_array = context.result_is_array;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(values) = &assertion.values {
        let resolved_field = assertion.field.as_deref().unwrap_or("");
        let resolved_name = field_resolver.resolve(resolved_field);
        let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
        let is_nullable = field_is_nullable;
        for val in values {
            let go_val = json_to_go(val);
            let field_for_contains =
                contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
            if is_nullable {
                let _ = writeln!(
                    out_ref,
                    "\tif {field_expr} == nil || !strings.Contains({field_for_contains}, {go_val}) {{"
                );
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
    }
}

pub(super) fn render_not_contains(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let result_is_array = context.result_is_array;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_data_interface = target.field_is_data_interface;

    for expected in assertion.expected_values() {
        let go_val = json_to_go(expected);
        let resolved_field = assertion.field.as_deref().unwrap_or("");
        let resolved_name = field_resolver.resolve(resolved_field);
        let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
        let is_nullable = field_is_nullable;
        let field_for_contains =
            contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
        let condition = if is_nullable {
            format!("{field_expr} != nil && strings.Contains({field_for_contains}, {go_val})")
        } else {
            format!("strings.Contains({field_for_contains}, {go_val})")
        };
        let _ = writeln!(out_ref, "\tif {condition} {{");
        let _ = writeln!(
            out_ref,
            "\t\tt.Errorf(\"expected NOT to contain %s, got %v\", {go_val}, {field_expr})"
        );
        let _ = writeln!(out_ref, "\t}}");
    }
}

pub(super) fn render_contains_any(
    out_ref: &mut String,
    context: &AssertionRenderContext<'_>,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_resolver = context.field_resolver;
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_nullable = target.field_is_nullable;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(values) = &assertion.values {
        let resolved_field = assertion.field.as_deref().unwrap_or("");
        let resolved_name = field_resolver.resolve(resolved_field);
        let field_is_array = field_resolver.is_array(resolved_name);
        let is_nullable = field_is_nullable;
        let field_for_contains =
            contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
        let _ = writeln!(out_ref, "\t{{");
        let _ = writeln!(out_ref, "\t\tfound := false");
        for val in values {
            let go_val = json_to_go(val);
            let condition = if is_nullable {
                format!("{field_expr} != nil && strings.Contains({field_for_contains}, {go_val})")
            } else {
                format!("strings.Contains({field_for_contains}, {go_val})")
            };
            let _ = writeln!(out_ref, "\t\tif {condition} {{ found = true }}");
        }
        let _ = writeln!(out_ref, "\t\tif !found {{");
        let _ = writeln!(
            out_ref,
            "\t\t\tt.Errorf(\"expected to contain at least one of the specified values\")"
        );
        let _ = writeln!(out_ref, "\t\t}}");
        let _ = writeln!(out_ref, "\t}}");
    }
}

pub(super) fn render_starts_with(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(expected) = &assertion.value {
        let go_val = json_to_go(expected);
        let field_for_prefix = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
        let _ = writeln!(out_ref, "\tif !strings.HasPrefix({field_for_prefix}, {go_val}) {{");
        let _ = writeln!(
            out_ref,
            "\t\tt.Errorf(\"expected to start with %s, got %v\", {go_val}, {field_expr})"
        );
        let _ = writeln!(out_ref, "\t}}");
    }
}

pub(super) fn render_ends_with(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(expected) = &assertion.value {
        let go_val = json_to_go(expected);
        let field_for_suffix = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
        let _ = writeln!(out_ref, "\tif !strings.HasSuffix({field_for_suffix}, {go_val}) {{");
        let _ = writeln!(
            out_ref,
            "\t\tt.Errorf(\"expected to end with %s, got %v\", {go_val}, {field_expr})"
        );
        let _ = writeln!(out_ref, "\t}}");
    }
}

pub(super) fn render_matches_regex(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let field_is_data_interface = target.field_is_data_interface;

    if let Some(expected) = &assertion.value {
        let go_val = json_to_go(expected);
        let field_for_regex = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
        let _ = writeln!(
            out_ref,
            "\tassert.Regexp(t, {go_val}, {field_for_regex}, \"expected value to match regex\")"
        );
    }
}
