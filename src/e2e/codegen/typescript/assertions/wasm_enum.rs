use crate::e2e::field_access::{FieldResolver, WasmEnumRepresentation};
use crate::e2e::fixture::Assertion;

use super::json_to_js;

/// Render an enum-typed result assertion for the wasm binding.
///
/// ~keep A data-carrying enum field crosses `serde_wasm_bindgen`, so its JavaScript shape follows
/// serde: an external tag is the object's sole key, an internal or adjacent tag is the configured
/// discriminator property, and an untagged payload has no discriminator to assert. Unit-only enums
/// remain scalar strings. Unknown IR metadata preserves the prior scalar comparison.
pub(super) fn render_wasm_enum_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    field: &str,
    field_resolver: &FieldResolver,
) -> bool {
    match assertion.assertion_type.as_str() {
        "equals" => render_equals(out, assertion, field_expr, field, field_resolver),
        "not_empty" | "is_not_empty" => {
            out.push_str(&render(crate::alef_context! {
                kind => "presence",
                actual => field_expr,
            }));
            true
        }
        _ => false,
    }
}

fn render_equals(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    field: &str,
    field_resolver: &FieldResolver,
) -> bool {
    let Some(serde_json::Value::String(expected)) = &assertion.value else {
        return false;
    };
    let wire = field_resolver
        .enum_wire_value_for_variant(field, expected)
        .unwrap_or(expected);
    let representation = field_resolver.wasm_enum_representation(field);
    let carries_data = field_resolver.ir_enum_is_data_carrying(field).unwrap_or(false);
    if carries_data && representation == Some(WasmEnumRepresentation::Untagged) {
        out.push_str(&render(crate::alef_context! { kind => "untagged", field => field }));
        return true;
    }
    let actual = actual_expression(field_expr, representation, carries_data);
    let expected = json_to_js(&serde_json::Value::String(wire.to_string()));
    out.push_str(&render(crate::alef_context! {
        kind => "equals",
        actual => actual,
        expected => expected,
    }));
    true
}

fn actual_expression(
    field_expr: &str,
    representation: Option<WasmEnumRepresentation<'_>>,
    carries_data: bool,
) -> String {
    match representation {
        Some(WasmEnumRepresentation::External) if carries_data => {
            format!("(typeof {field_expr} === \"string\" ? {field_expr} : Object.keys({field_expr} ?? {{}})[0])")
        }
        Some(WasmEnumRepresentation::Tagged { tag }) if carries_data => {
            let tag = json_to_js(&serde_json::Value::String(tag.to_string()));
            format!("{field_expr}?.[{tag}]")
        }
        _ => field_expr.to_string(),
    }
}

fn render(context: minijinja::Value) -> String {
    crate::e2e::template_env::render("typescript/wasm_enum_assertion.jinja", context)
}
