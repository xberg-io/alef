mod list_payload;

use crate::codegen::naming::dart_tuple_field_identifier;
use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::super::values::{collection_element_type, render_expected_value};
use super::field_to_dart_accessor;

fn render_tagged_union_leaf_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    is_collection: bool,
    is_optional: bool,
    declared: Option<&crate::core::ir::TypeRef>,
) {
    let element = declared.and_then(collection_element_type);
    let value_type = if assertion.assertion_type == "contains" {
        element
    } else {
        declared
    };
    let value = assertion
        .value
        .as_ref()
        .map(|value| render_expected_value(value, value_type));
    match assertion.assertion_type.as_str() {
        "equals" | "field_equals" if assertion.value.as_ref().is_some_and(serde_json::Value::is_string) => {
            if let Some(value) = value {
                let _ = writeln!(out, "    expect({field_expr}.toString(), equals({value}.toString()));");
            }
        }
        "equals" | "field_equals" => {
            if let Some(value) = value {
                let _ = writeln!(out, "    expect({field_expr}, equals({value}));");
            }
        }
        "contains" => {
            if let Some(value) = value {
                let _ = writeln!(out, "    expect({field_expr}, contains({value}));");
            }
        }
        "contains_all" => {
            for item in assertion.values.iter().flatten() {
                let _ = writeln!(
                    out,
                    "    expect({field_expr}, contains({}));",
                    render_expected_value(item, element)
                );
            }
        }
        "not_empty" if is_collection => {
            let _ = writeln!(out, "    expect({field_expr}, isNotEmpty);");
        }
        "not_empty" => {
            let _ = writeln!(out, "    expect({field_expr}.toString(), isNotEmpty);");
        }
        _ => render_tagged_union_numeric_assertion(out, assertion, field_expr, is_optional, declared),
    }
}

fn render_tagged_union_numeric_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    is_optional: bool,
    declared: Option<&crate::core::ir::TypeRef>,
) {
    let length = || {
        if is_optional {
            format!("{field_expr}?.length ?? 0")
        } else {
            format!("{field_expr}.length")
        }
    };
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
                let _ = writeln!(out, "    expect({}, equals({count}));", length());
            }
        }
        "count_min" | "min_length" => {
            if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
                let _ = writeln!(out, "    expect({}, greaterThanOrEqualTo({count}));", length());
            }
        }
        "greater_than_or_equal" => {
            if let Some(value) = assertion
                .value
                .as_ref()
                .map(|value| render_expected_value(value, declared))
            {
                let _ = writeln!(out, "    expect({field_expr}, greaterThanOrEqualTo({value}));");
            }
        }
        other => {
            let reason = AssertionTypeSkip::DiscriminatedUnionAssertionTypeNotSupported.message(other);
            let _ = writeln!(out, "    // skipped: {reason}");
        }
    }
}

fn dart_payload_accessor(field_resolver: &FieldResolver, owner: &str, path: &str) -> String {
    let segments: Vec<&str> = path.split('.').filter(|segment| !segment.is_empty()).collect();
    let mut accessor = String::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            let prefix = segments[..index].join(".");
            accessor.push_str(if field_resolver.ir_field_is_optional_from(owner, &prefix) {
                "?."
            } else {
                "."
            });
        }
        accessor.push_str(&field_to_dart_accessor(segment));
    }
    accessor
}

fn dart_union_payload_accessor(payload_field: &str) -> String {
    let tuple_index = payload_field.strip_prefix('_').unwrap_or(payload_field);
    if !tuple_index.is_empty() && tuple_index.chars().all(|character| character.is_ascii_digit()) {
        dart_tuple_field_identifier(tuple_index)
    } else {
        field_to_dart_accessor(payload_field)
    }
}

fn narrow_tagged_union_expression(
    field_resolver: &FieldResolver,
    container: String,
    union_type: String,
    variant: String,
    suffix: String,
) -> Option<(String, String, String, String)> {
    let (payload_field, payload_type) = field_resolver.union_variant_payload(&union_type, &variant)?;
    let narrowed = format!(
        "({container} as {union_type}_{variant}).{}",
        dart_union_payload_accessor(payload_field)
    );
    let Some((prefix, next_union, next_variant, next_suffix)) =
        field_resolver.ir_tagged_union_split_from(payload_type, &suffix)
    else {
        let accessor = dart_payload_accessor(field_resolver, payload_type, &suffix);
        let expression = if accessor.is_empty() {
            narrowed
        } else {
            format!("{narrowed}.{accessor}")
        };
        return Some((expression, union_type, variant, suffix));
    };
    let prefix_accessor = dart_payload_accessor(field_resolver, payload_type, &prefix);
    let next_container = if prefix_accessor.is_empty() {
        narrowed
    } else {
        format!("{narrowed}.{prefix_accessor}")
    };
    narrow_tagged_union_expression(field_resolver, next_container, next_union, next_variant, next_suffix)
}

pub(super) fn try_render_tagged_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    field: &str,
) -> bool {
    if let Some(rendered) = list_payload::render(assertion, field_resolver, result_var, field) {
        out.push_str(&rendered);
        return true;
    }
    let Some((prefix, union_type, variant, suffix)) = field_resolver.ir_tagged_union_split(field) else {
        return false;
    };
    if field_resolver.union_variant_payload(&union_type, &variant).is_none() {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::CrossesTaggedUnionBoundaryInDart.message(field)
        );
        return true;
    }
    let container = field_resolver.accessor(&prefix, "dart", result_var);
    let Some((field_expr, leaf_union, leaf_variant, leaf_suffix)) =
        narrow_tagged_union_expression(field_resolver, container, union_type, variant, suffix)
    else {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::CrossesTaggedUnionBoundaryInDart.message(field)
        );
        return true;
    };
    let is_collection =
        field_resolver.union_variant_field_is_collection_by_type(&leaf_union, &leaf_variant, &leaf_suffix);
    let is_optional = field_resolver.union_variant_field_is_optional(&leaf_union, &leaf_variant, &leaf_suffix);
    render_tagged_union_leaf_assertion(
        out,
        assertion,
        &field_expr,
        is_collection,
        is_optional,
        field_resolver.target_value_type(field),
    );
    true
}

#[cfg(test)]
mod list_payload_tests;
