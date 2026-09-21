//! C# JSON literal and sealed-display rendering helpers.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::escape::escape_csharp;

mod field_types;
pub(crate) use field_types::{
    json_array_to_csharp_list, resolve_csharp_field_element_type_from_struct, resolve_csharp_field_type_from_struct,
};

/// The call's declared Rust result type, resolved from the IR itself (not a hand-configured
/// override) — anchors `FieldResolver`'s IR-derived enum classification (`with_ir_enum_map`) at
/// the exact struct/enum this call returns, mirroring the rust e2e generator's fix for the same
/// defect shape (a field name that means different things on different types). ~keep
pub(super) fn resolve_csharp_call_root_type(
    call_config: &crate::e2e::config::CallConfig,
    type_defs: &[crate::core::ir::TypeDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Option<String> {
    let call_ir = CallIr { functions, type_defs };
    resolve_declared_result_type(call_config, "csharp", call_ir)
}

/// Render a C# sealed-union display helper for assert_enum_fields.
/// Pattern-matches on variants from the IR and returns a displayable string.
pub(super) fn render_sealed_display(
    type_name: &str,
    enum_def: &crate::core::ir::EnumDef,
    type_defs: &[crate::core::ir::TypeDef],
    namespace: &str,
) -> String {
    let header = hash::e2e_header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("namespace {namespace}.E2e;\n\n"));
    out.push_str(&format!(
        "/// <summary>\n/// Helper class for extracting display strings from {type_name} sealed interface.\n /// </summary>\n"
    ));
    out.push_str(&format!("internal static class {type_name}Display\n"));
    out.push_str("{\n");
    out.push_str(&format!(
        "    internal static string ToDisplayString({type_name}? value)\n"
    ));
    out.push_str("    {\n");
    out.push_str("        if (value == null) return \"\";\n");
    out.push_str("        return value switch\n");
    out.push_str("        {\n");

    for variant in &enum_def.variants {
        let variant_name = &variant.name;
        // Determine the display string for this variant's arm.
        // Tuple variants with one field whose resolved struct type has a `format`
        // field return the inner `.Value.Format` — this gives the actual format
        // string (e.g. "PNG") rather than the generic variant label (e.g. "image").
        let has_format_field = variant.is_tuple && variant.fields.len() == 1 && {
            let field_type_name = match &variant.fields[0].ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            };
            field_type_name.is_some_and(|tn| {
                type_defs
                    .iter()
                    .find(|td| td.name == tn)
                    .is_some_and(|td| td.fields.iter().any(|f| f.name == "format"))
            })
        };

        let display = if has_format_field {
            "i.Value.Format".to_string()
        } else {
            // Routed through the same seam the production `json_name` discriminator uses
            // (`backends/csharp/gen_bindings/enums.rs`), so this display value cannot drift
            // from `serde_rename_all` or fall back to lowercasing an explicit `serde_rename`.
            let wire_name = crate::codegen::naming::wire_variant_value(
                variant_name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            format!("\"{wire_name}\"")
        };

        let binding = if has_format_field {
            format!("{type_name}.{variant_name} i")
        } else {
            format!("{type_name}.{variant_name}")
        };

        out.push_str(&format!("            {binding} => {display},\n"));
    }

    out.push_str("            _ => \"unknown\",\n");
    out.push_str("        };\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

/// Merge per-call C# `enum_fields` keys with the global file-level `fields_enum` set so
/// call-specific enum-typed result fields (e.g. BatchObject's `status` → BatchStatus) trigger
/// enum coercion in assertions even when the global set does not list them. The file-level
/// `enum_fields` argument carries the default-call's override; `cs_overrides.enum_fields`
/// carries the per-fixture-call's override (e.g. retrieve_batch.overrides.csharp.enum_fields).
///
/// Must be the EFFECTIVE set: a per-call `fields_enum` REPLACES the global rather than merging,
/// so reading the global directly discards the override outright. Every other language resolves
/// this through the accessor; C# was the only one reading the raw global. This is a different
/// axis from the per-language `enum_fields` override maps merged in just below. ~keep
pub(super) fn effective_csharp_enum_fields(
    e2e_config: &crate::e2e::config::E2eConfig,
    call_config: &crate::e2e::config::CallConfig,
    enum_fields: &std::collections::HashMap<String, String>,
    cs_overrides: Option<&crate::e2e::config::CallOverride>,
) -> std::collections::HashSet<String> {
    let mut effective_enum_fields: std::collections::HashSet<String> =
        e2e_config.effective_fields_enum(call_config).clone();
    for k in enum_fields.keys() {
        effective_enum_fields.insert(k.clone());
    }
    if let Some(o) = cs_overrides {
        for k in o.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }
    effective_enum_fields
}

/// Above this many elements, a C# collection literal (`new[] { ... }`, `new List<T>() { ... }`)
/// is wrapped one element per line instead of emitted inline. Fixture-driven catalogs can carry
/// hundreds or thousands of elements; inlining those onto one line produces a single unwrapped
/// line tens of thousands of characters long that the formatter must reflow from scratch (#365).
/// No inline literal in the current test suite exceeds 2 elements, so this stays well clear of
/// every existing exact-output assertion. ~keep
pub(super) const CSHARP_COLLECTION_INLINE_LIMIT: usize = 8;

/// Render a C# collection literal (`new[] { .. }`, `new List<T>() { .. }`, ...) from a
/// constructor `prefix` and already-rendered element expressions. Stays on one line for small
/// literals; above [`CSHARP_COLLECTION_INLINE_LIMIT`] elements, wraps one element per line via
/// `csharp/wrapped_collection_literal.jinja` so the formatter receives output that is already
/// close to its final shape rather than one enormous line to reflow.
pub(super) fn render_collection_literal(prefix: &str, items: Vec<String>) -> String {
    if items.len() <= CSHARP_COLLECTION_INLINE_LIMIT {
        return format!("{prefix} {{ {} }}", items.join(", "));
    }
    crate::e2e::template_env::render(
        "csharp/wrapped_collection_literal.jinja",
        minijinja::context! { prefix => prefix, items => items },
    )
    .trim_end_matches('\n')
    .to_string()
}

/// Convert a `serde_json::Value` to a C# literal string.
pub(super) fn json_to_csharp(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_csharp(s)),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                format!("{}d", n)
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_csharp).collect();
            render_collection_literal("new[]", items)
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_csharp(&json_str))
        }
    }
}
