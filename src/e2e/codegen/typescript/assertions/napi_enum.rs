use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::json_to_js;

/// Render an enum-typed result assertion for the node (napi) binding.
///
/// ~keep A tagged data enum crosses napi as an internally-tagged object — `{ type: "Function" }`
/// for a unit variant, the discriminant property named by the enum's `#[serde(tag = "...")]` or
/// `"type"` by default. Comparing that object as a scalar is what the generic path did, and
/// `String({ type: "Function" })` is `"[object Object]"`, so the assertion could never hold. A
/// `#[napi(string_enum)]` stays a scalar string and never reaches here — `napi_tagged_object_
/// discriminant` answers `None` for it, leaving the ordinary comparison in place.
pub(super) fn render_napi_enum_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    field: &str,
    field_resolver: &FieldResolver,
) -> bool {
    let Some(tag) = field_resolver.napi_tagged_object_discriminant(field) else {
        return false;
    };
    match assertion.assertion_type.as_str() {
        "equals" => {
            let Some(serde_json::Value::String(expected)) = &assertion.value else {
                return false;
            };
            let wire = field_resolver
                .enum_wire_value_for_variant(field, expected)
                .unwrap_or(expected);
            let tag_key = json_to_js(&serde_json::Value::String(tag.to_string()));
            out.push_str(&render(minijinja::context! {
                kind => "equals",
                actual => format!("{field_expr}?.[{tag_key}]"),
                expected => json_to_js(&serde_json::Value::String(wire.to_string())),
            }));
            true
        }
        "not_empty" | "is_not_empty" => {
            let tag_key = json_to_js(&serde_json::Value::String(tag.to_string()));
            out.push_str(&render(minijinja::context! {
                kind => "presence",
                actual => format!("{field_expr}?.[{tag_key}]"),
            }));
            true
        }
        _ => false,
    }
}

fn render(context: minijinja::Value) -> String {
    crate::e2e::template_env::render("typescript/napi_enum_assertion.jinja", context)
}
