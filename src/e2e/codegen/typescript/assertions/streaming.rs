use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::fixture::Assertion;

use super::json_to_js;

pub(super) fn render(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    streaming_item_enum: Option<&crate::core::ir::EnumDef>,
) -> bool {
    let event_expression = streaming_item_enum.and_then(|enum_def| event_variant_accessor(field, "chunks", enum_def));
    let expression = event_expression.or_else(|| {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(field, "node", "chunks")
    });
    let Some(expression) = expression else {
        // ~keep An unsupported stream predicate must remain visible to the skip ledger.
        out.push_str(&format!(
            "    // skipped: {}\n",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        ));
        return true;
    };

    match assertion.assertion_type.as_str() {
        "count_min" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}.length).toBeGreaterThanOrEqual({value});\n")
        }),
        "count_equals" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}.length).toBe({value});\n")
        }),
        "equals" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}).toBe({value});\n")
        }),
        "not_empty" => out.push_str(&crate::e2e::template_env::render(
            "typescript/assertion.jinja",
            crate::alef_context! {
                assertion_type => "not_empty",
                field_expr => expression,
                field_is_optional => false,
            },
        )),
        "is_empty" => out.push_str(&format!("    expect({expression}).toBeFalsy();\n")),
        "is_true" => out.push_str(&format!("    expect({expression}).toBe(true);\n")),
        "is_false" => out.push_str(&format!("    expect({expression}).toBe(false);\n")),
        "greater_than" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}).toBeGreaterThan({value});\n")
        }),
        "greater_than_or_equal" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}).toBeGreaterThanOrEqual({value});\n")
        }),
        "contains" => render_value(out, assertion, field, |value| {
            format!("    expect({expression}).toContain({value});\n")
        }),
        _ => out.push_str(&format!(
            "{}\n",
            streaming_assertion_type_skip_line("    ", "//", field, &assertion.assertion_type)
        )),
    }
    true
}

fn render_value(out: &mut String, assertion: &Assertion, field: &str, render: impl FnOnce(String) -> String) {
    if let Some(value) = &assertion.value {
        out.push_str(&render(json_to_js(value)));
    } else {
        out.push_str(&format!(
            "{}\n",
            streaming_assertion_value_skip_line("    ", "//", field, &assertion.assertion_type)
        ));
    }
}

fn event_variant_accessor(field: &str, chunks: &str, enum_def: &crate::core::ir::EnumDef) -> Option<String> {
    let variant_name = match field {
        "stream.has_page_event" => "Page",
        "stream.has_error_event" => "Error",
        "stream.has_complete_event" => "Complete",
        _ => return None,
    };
    let variant = enum_def.variants.iter().find(|variant| variant.name == variant_name)?;
    // `enum_def` here is always a streaming chunk-event enum (`Page`/`Error`/`Complete`), never
    // an internally-tagged enum with a newtype payload serde would flatten -- there is no IR
    // type list threaded this deep into assertion rendering to check that resolution-aware
    // condition properly, so this narrow call site accepts the same fixed set of variant names
    // as its only guard instead. (~keep)
    if let Some(value) = crate::backends::napi::string_enum_variant_js_value(enum_def, variant_name, &[]) {
        let value = serde_json::to_string(&value).ok()?;
        return Some(format!(
            "{chunks}.some((event: {}) => event === {value})",
            enum_def.name
        ));
    }
    if !crate::backends::napi::is_tagged_data_enum(enum_def, &[]) {
        return None;
    }
    let tag_field = serde_json::to_string(crate::backends::napi::tagged_enum_discriminant_js_name(enum_def)).ok()?;
    let tag_value = crate::codegen::naming::wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    );
    let tag_value = serde_json::to_string(&tag_value).ok()?;
    Some(format!(
        "{chunks}.some((event: {}) => event[{tag_field}] === {tag_value})",
        enum_def.name
    ))
}
