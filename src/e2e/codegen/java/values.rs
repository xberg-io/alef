use crate::e2e::escape::escape_java;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

/// Check if a type name is a numeric type hint (f32, float, etc.) vs. a complex type name.
pub(super) fn is_numeric_type_hint(ty: &str) -> bool {
    matches!(ty, "f32" | "f64" | "float" | "double" | "Float" | "Double")
}

/// Check if a type name is a Java built-in type that doesn't need an import.
pub(super) fn is_java_builtin_type(ty: &str) -> bool {
    matches!(
        ty,
        "String" | "Boolean" | "Integer" | "Long" | "Double" | "Float" | "Byte" | "Short" | "Character" | "Void"
    )
}

/// Emit a Java list of deserialized objects via JsonUtil.
/// E.g., `[{"type": "click", ...}, ...]` becomes `java.util.Arrays.asList(JsonUtil.fromJson(...))`.
pub(super) fn emit_java_object_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        if items.is_empty() {
            return "java.util.List.of()".to_string();
        }
        let item_strs: Vec<String> = items
            .iter()
            .map(|item| {
                let json_str = serde_json::to_string(item).unwrap_or_default();
                let escaped = escape_java(&json_str);
                format!("JsonUtil.fromJson(\"{escaped}\", {elem_type}.class)")
            })
            .collect();
        format!("java.util.Arrays.asList({})", item_strs.join(", "))
    } else {
        "java.util.List.of()".to_string()
    }
}

/// Convert a `serde_json::Value` to a Java literal string.
pub(super) fn json_to_java(value: &serde_json::Value) -> String {
    json_to_java_typed(value, None)
}

/// Convert a JSON value to a Java literal, optionally overriding number type for array elements.
/// `element_type` controls how numeric array elements are emitted: "f32" -> `1.0f`, otherwise `1.0d`.
pub(super) fn json_to_java_typed(value: &serde_json::Value, element_type: Option<&str>) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_java(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                match element_type {
                    Some("f32" | "float" | "Float") => format!("{}f", n),
                    _ => format!("{}d", n),
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, element_type)).collect();
            format!("java.util.List.of({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_java(&json_str))
        }
    }
}

/// Generate a Java builder expression for a JSON object.
/// E.g., `obj = {"language": "abl", "chunk_max_size": 50}`
/// becomes: `TypeName.builder().withLanguage("abl").withChunkMaxSize(50L).build()`
///
/// For enums: emit `EnumType.VariantName` (detected via camelCase lookup in enum_fields)
/// For strings and bools: use the value directly
/// For plain numbers: emit the literal with type suffix (long uses L, double uses d)
/// For nested objects: recurse with Options suffix
/// When `nested_types_optional` is false, nested builders are passed directly without
/// Optional.of() wrapping, allowing non-optional nested config types.
pub(super) fn java_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_fields: &std::collections::HashSet<String>,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    path_fields: &[String],
) -> String {
    let mut expr = format!("{}.builder()", type_name);
    for (key, val) in obj {
        // Convert snake_case key to camelCase for method name
        let camel_key = key.to_lower_camel_case();
        let method_name = format!("with{}", camel_key.to_upper_camel_case());

        let java_val = match val {
            serde_json::Value::String(s) => {
                // Check if this field is an enum type by checking enum_fields.
                // Infer enum type name from camelCase field name by converting to UpperCamelCase.
                if enum_fields.contains(&camel_key) {
                    // Enum field: infer type name from field name (e.g., "codeBlockStyle" -> "CodeBlockStyle")
                    let enum_type_name = camel_key.to_upper_camel_case();
                    let variant_name = s.to_upper_camel_case();
                    format!("{}.{}", enum_type_name, variant_name)
                } else if path_fields.contains(key) {
                    // Path field: wrap in Optional.of(java.nio.file.Path.of(...))
                    format!("Optional.of(java.nio.file.Path.of(\"{}\"))", escape_java(s))
                } else {
                    // String field: emit as a quoted literal
                    format!("\"{}\"", escape_java(s))
                }
            }
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Number(n) => {
                // Number field: emit literal with type suffix.
                // Java records/classes use either `long` (primitive, not nullable) or
                // `Optional<Long>` (nullable). The codegen wraps in `Optional.of(...)`
                // by default since most options builder fields are Optional. Calls that
                // use primitive builder fields can opt into bare values by setting
                // `nested_types_optional = false`.
                let camel_key = key.to_lower_camel_case();
                let is_plain_field = matches!(camel_key.as_str(), "listIndentWidth" | "wrapWidth");
                let is_primitive_builder = !nested_types_optional;

                if is_plain_field || is_primitive_builder {
                    // Plain numeric field: no Optional wrapper
                    if n.is_f64() {
                        format!("{}d", n)
                    } else {
                        format!("{}L", n)
                    }
                } else {
                    // Optional numeric field: wrap in Optional.of()
                    if n.is_f64() {
                        format!("Optional.of({}d)", n)
                    } else {
                        format!("Optional.of({}L)", n)
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, None)).collect();
                format!("java.util.List.of({})", items.join(", "))
            }
            serde_json::Value::Object(nested) => {
                // Recurse with the type from nested_types mapping, or default to snake_case -> PascalCase + "Options".
                let nested_type = nested_types
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("{}Options", key.to_upper_camel_case()));
                let inner = java_builder_expression(
                    nested,
                    &nested_type,
                    enum_fields,
                    nested_types,
                    nested_types_optional,
                    &[],
                );
                // Top-level config builders usually declare nested record fields as
                // `Optional<T>`. Calls with non-optional nested config builders can opt
                // into passing the bare builder result.
                let is_primitive_builder = !nested_types_optional;
                if is_primitive_builder || !nested_types_optional {
                    inner
                } else {
                    format!("Optional.of({inner})")
                }
            }
        };
        expr.push_str(&format!(".{}({})", method_name, java_val));
    }
    expr.push_str(".build()");
    expr
}

/// Recursively collect enum types and nested option types used in a builder expression.
/// Enums are keyed in the enum_fields map by camelCase names (e.g., "codeBlockStyle" -> "CodeBlockStyle").
#[allow(dead_code)]
pub(super) fn collect_enum_and_nested_types(
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        // enum_fields is keyed by camelCase, not snake_case.
        let camel_key = key.to_lower_camel_case();
        if let Some(enum_type) = enum_fields.get(&camel_key) {
            // Add the enum type from the mapping (e.g., "CodeBlockStyle").
            types_out.insert(enum_type.clone());
        }
        // Recurse into nested objects to find their nested enum types.
        if let Some(nested) = val.as_object() {
            collect_enum_and_nested_types(nested, enum_fields, types_out);
        }
    }
}

pub(super) fn collect_nested_type_names(
    obj: &serde_json::Map<String, serde_json::Value>,
    nested_types: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        if let Some(type_name) = nested_types.get(key.as_str()) {
            types_out.insert(type_name.clone());
        }
        if let Some(nested) = val.as_object() {
            collect_nested_type_names(nested, nested_types, types_out);
        }
    }
}
