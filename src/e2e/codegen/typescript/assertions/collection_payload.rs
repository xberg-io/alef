use crate::e2e::field_access::{FieldResolver, WireAccessStep};
use crate::e2e::fixture::Assertion;

pub(super) fn render(out: &mut String, assertion: &Assertion, result: &str, resolver: &FieldResolver) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Some((prefix, owner, variant, suffix)) = resolver.ir_tagged_union_split(resolver.resolve(field)) else {
        return false;
    };
    if !resolver.union_variant_payload_is_collection(&owner, &variant) {
        return false;
    }
    let Some(steps) = resolver.enum_payload_wire_steps(&owner, &variant, &suffix) else {
        unsupported(out, field);
        return true;
    };
    let Some(keys) = resolver.enum_payload_wire_prefix(&owner, &variant) else {
        return false;
    };
    // ~keep A text-projected union reads through its lossless `<field>Value` getter: the plain
    // getter returns the display text, which has already lost the parts.
    let mut payload = resolver
        .wasm_text_value_accessor(&prefix, &owner, result)
        .unwrap_or_else(|| resolver.accessor(&prefix, "typescript", result));
    for key in keys {
        payload.push_str(&format!("[{}]", super::json_to_js(&serde_json::Value::String(key))));
    }
    let mut expression = guard_array(&payload);
    for step in steps {
        expression = match step {
            WireAccessStep::Field(key) => member(&expression, &key),
            WireAccessStep::Index(index) => format!("{}[{index}]", guard_array(&expression)),
        };
    }
    let size = size_expression(&expression, &assertion.assertion_type);
    out.push_str(&crate::e2e::template_env::render(
        "typescript/collection_payload.jinja",
        crate::alef_context! {
            payload => payload, expression => expression, size => size,
            assertion_type => &assertion.assertion_type,
            expected => assertion.value.as_ref().map(super::json_to_js),
            field => field,
        },
    ));
    true
}

#[cfg(test)]
#[path = "collection_payload/tests.rs"]
mod tests;

fn unsupported(out: &mut String, field: &str) {
    out.push_str(&crate::e2e::template_env::render(
        "typescript/collection_payload.jinja",
        crate::alef_context! { assertion_type => "unsupported", field => field },
    ));
}

fn guard_array(value: &str) -> String {
    crate::e2e::template_env::render(
        "typescript/json_guard.ts.jinja",
        crate::alef_context! { guard => "array", value => value },
    )
    .trim()
    .to_string()
}

fn member(value: &str, key: &str) -> String {
    crate::e2e::template_env::render(
        "typescript/json_member.ts.jinja",
        crate::alef_context! { kind => "object", value => value, key => super::json_to_js(&key.into()) },
    )
    .trim()
    .to_string()
}

fn size_expression(expression: &str, assertion: &str) -> Option<String> {
    match assertion {
        "count_equals" | "count_min" => Some(format!("{}.length", guard_array(expression))),
        "not_empty" | "is_empty" => Some(
            crate::e2e::template_env::render(
                "typescript/json_guard.ts.jinja",
                crate::alef_context! { guard => "sized", value => expression },
            )
            .trim()
            .to_string(),
        ),
        _ => None,
    }
}
