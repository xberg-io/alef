use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

pub(super) fn render(
    assertion: &Assertion,
    resolver: &FieldResolver,
    result: &str,
    android: bool,
    field: &str,
) -> Option<String> {
    let (prefix, owner, variant, suffix) = resolver.ir_tagged_union_split(field)?;
    if !resolver.union_variant_payload_is_collection(&owner, &variant) {
        return None;
    }
    if !android {
        return render_json(assertion, resolver, result, (&prefix, &owner, &variant, &suffix));
    }
    let (payload_name, element_type) = resolver.union_variant_payload(&owner, &variant)?;
    let style = if android { "kotlin_android" } else { "kotlin" };
    let container = if prefix.is_empty() {
        result.to_string()
    } else {
        resolver.accessor(&prefix, style, result)
    };
    let member = crate::backends::kotlin::kotlin_field_name_with_type(payload_name, 0, None, &variant, 1);
    let payload = format!("listVariant.{member}");
    let expression = if suffix.is_empty() {
        payload
    } else {
        indexed_leaf(resolver, element_type, &suffix, &payload, style)?
    };
    let expected = assertion.value.as_ref().map(super::super::values::json_to_kotlin)?;
    let kind = assertion.assertion_type.as_str();
    if !matches!(kind, "count_equals" | "equals" | "field_equals") {
        return None;
    }
    Some(crate::e2e::template_env::render(
        "kotlin/list_variant_assertion.jinja",
        crate::alef_context! { container => container, owner => owner, variant => variant,
        expression => expression, expected => expected, count => kind == "count_equals" },
    ))
}

/// The binding types that the typed (Android) rendering of `field` names: the union and the
/// union type of its list elements. The Android test package is a child of the binding package,
/// so it must import each of them. ~keep
pub(in crate::e2e::codegen::kotlin) fn android_imports(resolver: &FieldResolver, field: &str) -> Vec<String> {
    let Some((_, owner, variant, _)) = resolver.ir_tagged_union_split(field) else {
        return Vec::new();
    };
    let Some((_, element_type)) = resolver.union_variant_payload(&owner, &variant) else {
        return Vec::new();
    };
    if !resolver.union_variant_payload_is_collection(&owner, &variant) {
        return Vec::new();
    }
    let mut types = vec![owner];
    if resolver.enum_definition(element_type).is_some() {
        types.push(element_type.to_string());
    }
    types
}

fn indexed_leaf(resolver: &FieldResolver, owner: &str, suffix: &str, payload: &str, style: &str) -> Option<String> {
    let end = suffix.find(']')?;
    let element = resolver.element_accessor(&suffix[..=end], style, payload);
    let tail = suffix[end + 1..].trim_start_matches('.');
    if resolver.enum_definition(owner).is_none() {
        return Some(resolver.element_accessor(tail, style, &element));
    }
    let (head, remainder) = tail.split_once('.').unwrap_or((tail, ""));
    let metadata = resolver.enum_field_metadata(owner, head)?;
    let member = crate::backends::kotlin::kotlin_field_name_with_type(
        &metadata.field,
        0,
        metadata.next_type.as_deref(),
        &metadata.variant,
        1,
    );
    let narrowed = format!("({element} as {owner}.{}).{member}", metadata.variant);
    Some(if remainder.is_empty() {
        narrowed
    } else {
        resolver.element_accessor(remainder, style, &narrowed)
    })
}

fn render_json(
    assertion: &Assertion,
    resolver: &FieldResolver,
    result: &str,
    crossing: (&str, &str, &str, &str),
) -> Option<String> {
    let (prefix, owner, variant, suffix) = crossing;
    if !resolver.enum_definition(owner)?.serde_untagged {
        return None;
    }
    let container = if prefix.is_empty() {
        result.to_string()
    } else {
        resolver.accessor(prefix, "kotlin", result)
    };
    let payload = format!("kotlin.test.assertNotNull({container}).value()");
    let mut expression = payload.clone();
    for step in resolver.enum_payload_wire_steps(owner, variant, suffix)? {
        match step {
            crate::e2e::field_access::WireAccessStep::Field(key) => {
                let literal = super::super::values::json_to_kotlin(&serde_json::Value::String(key));
                expression.push_str(&format!(".get({literal})"));
            }
            crate::e2e::field_access::WireAccessStep::Index(index) => expression.push_str(&format!(".get({index})")),
        }
    }
    let kind = assertion.assertion_type.as_str();
    if !matches!(kind, "count_equals" | "equals" | "field_equals") {
        return None;
    }
    let expected_value = assertion.value.as_ref()?;
    if kind != "count_equals" && !expected_value.is_string() {
        return None;
    }
    Some(crate::e2e::template_env::render(
        "kotlin/list_variant_json_assertion.jinja",
        crate::alef_context! {
            payload => payload, expression => expression, expected => super::super::values::json_to_kotlin(expected_value),
            count => kind == "count_equals",
        },
    ))
}
