//! C# JSON literal and sealed-display rendering helpers.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::escape::escape_csharp;
use std::fmt::Write as FmtWrite;

/// Render a C# sealed-union display helper for assert_enum_fields.
/// Pattern-matches on variants from the IR and returns a displayable string.
pub(super) fn render_sealed_display(
    type_name: &str,
    enum_def: &crate::core::ir::EnumDef,
    type_defs: &[crate::core::ir::TypeDef],
    namespace: &str,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
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
            // Use the serde rename when present; otherwise lowercase the variant name.
            let serde_name = variant
                .serde_rename
                .as_deref()
                .unwrap_or(variant_name.as_str())
                .to_lowercase();
            format!("\"{serde_name}\"")
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
            format!("new[] {{ {} }}", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_csharp(&json_str))
        }
    }
}
