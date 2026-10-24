//! Assertions that step through an untagged union into a list payload, such as
//! `choices[0].message.content.parts[0].text`.
//!
//! The Java binding lowers a `#[serde(untagged)]` union with data variants to a wrapper class
//! that holds the wire JSON and exposes it through `value()`. The wrapper has no accessor
//! for a variant, so the path below the union is read from the JSON tree.

use crate::e2e::field_access::{FieldResolver, WireAccessStep};
use crate::e2e::fixture::Assertion;

use super::values::json_to_java;

/// Render `assertion` over the JSON tree of the union wrapper. Return `false`, and write
/// nothing, when the field does not cross an untagged union into a list payload.
pub(super) fn try_render(out: &mut String, assertion: &Assertion, result_var: &str, resolver: &FieldResolver) -> bool {
    let Some(field) = assertion.field.as_deref().filter(|f| !f.is_empty()) else {
        return false;
    };
    let kind = assertion.assertion_type.as_str();
    let Some(expected) = assertion.value.as_ref() else {
        return false;
    };
    if !matches!(kind, "count_equals" | "equals") || (kind == "equals" && !expected.is_string()) {
        return false;
    }
    if kind == "count_equals" && !expected.is_u64() {
        return false;
    }
    let Some((prefix, owner, variant, suffix)) = resolver.ir_tagged_union_split(resolver.resolve(field)) else {
        return false;
    };
    let Some(definition) = resolver.enum_definition(&owner) else {
        return false;
    };
    if !definition.serde_untagged || !resolver.union_variant_payload_is_collection(&owner, &variant) {
        return false;
    }
    if prefix.is_empty() {
        return false;
    }
    let Some(steps) = resolver.enum_payload_wire_steps(&owner, &variant, &suffix) else {
        return false;
    };
    let container = resolver.accessor(&prefix, "java", result_var);
    let payload = format!("{container}.value()");
    let mut expression = payload.clone();
    for step in steps {
        match step {
            WireAccessStep::Field(key) => {
                let literal = json_to_java(&serde_json::Value::String(key));
                expression.push_str(&format!(".get({literal})"));
            }
            WireAccessStep::Index(index) => expression.push_str(&format!(".get({index})")),
        }
    }
    out.push_str(&crate::e2e::template_env::render(
        "java/list_variant_json_assertion.jinja",
        crate::alef_context! {
            container => container, payload => payload, expression => expression,
            expected => json_to_java(expected), count => kind == "count_equals",
        },
    ));
    true
}
