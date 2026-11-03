use crate::codegen::naming::{dart_tuple_field_identifier, public_field_name};
use crate::core::config::Language;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

pub(super) fn render(assertion: &Assertion, resolver: &FieldResolver, result: &str, field: &str) -> Option<String> {
    let (prefix, owner, variant, suffix) = resolver.ir_tagged_union_split(field)?;
    if !resolver.union_variant_payload_is_collection(&owner, &variant) {
        return None;
    }
    let (payload_field, element_type) = resolver.union_variant_payload(&owner, &variant)?;
    let container = if prefix.is_empty() {
        result.to_string()
    } else {
        resolver.accessor(&prefix, "dart", result)
    };
    let member = dart_tuple_field_identifier(payload_field.trim_start_matches('_'));
    let payload = format!("({container} as {owner}_{variant}).{member}");
    let expression = if suffix.is_empty() {
        payload
    } else {
        indexed_leaf(resolver, element_type, &suffix, &payload)?
    };
    let kind = assertion.assertion_type.as_str();
    let declared = if kind == "count_equals" {
        None
    } else {
        resolver.target_value_type(field)
    };
    let expected = assertion
        .value
        .as_ref()
        .map(|value| super::super::super::values::render_expected_value(value, declared))?;
    if !matches!(kind, "count_equals" | "equals" | "field_equals") {
        return None;
    }
    Some(crate::e2e::template_env::render(
        "dart/list_variant_assertion.jinja",
        crate::alef_context! {
            container => container, owner => owner, variant => variant,
            expression => expression, expected => expected, count => kind == "count_equals",
        },
    ))
}

fn indexed_leaf(resolver: &FieldResolver, owner: &str, suffix: &str, payload: &str) -> Option<String> {
    let end = suffix.find(']')?;
    let element = resolver.element_accessor(&suffix[..=end], "dart", payload);
    let tail = suffix[end + 1..].trim_start_matches('.');
    if resolver.enum_definition(owner).is_none() {
        return Some(resolver.element_accessor(tail, "dart", &element));
    }
    let (head, remainder) = tail.split_once('.').unwrap_or((tail, ""));
    let metadata = resolver.enum_field_metadata(owner, head)?;
    let member = public_field_name(Language::Dart, &metadata.field, None);
    let narrowed = format!("({element} as {owner}_{}).{member}", metadata.variant);
    Some(if remainder.is_empty() {
        narrowed
    } else {
        resolver.element_accessor(remainder, "dart", &narrowed)
    })
}
