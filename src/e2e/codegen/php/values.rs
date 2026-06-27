//! PHP e2e PHP-literal rendering helpers.

use crate::core::ir::TypeRef;
use crate::e2e::escape::escape_php;
use heck::ToLowerCamelCase;

/// Render a PHP object-array for typed array args.
///
/// Emit PHP object array elements for a typed `json_object` array.
pub(super) fn emit_php_object_array(arr: &serde_json::Value, elem_type: &str) -> String {
    emit_php_object_array_with_mock_base(arr, elem_type, None)
}

/// Render a PHP object array and optionally replace `$mock_url` at runtime.
pub(super) fn emit_php_object_array_with_mock_base(
    arr: &serde_json::Value,
    elem_type: &str,
    mock_base_var: Option<&str>,
) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let Some(obj) = item.as_object() {
                    let json_str = serde_json::to_string(&serde_json::Value::Object(obj.clone()))
                        .unwrap_or_else(|_| "{}".to_string());
                    let php_literal = json_str.replace('\\', "\\\\").replace('\'', "\\'");
                    if let Some(base_var) = mock_base_var.filter(|_| {
                        crate::e2e::codegen::value_contains_mock_url_placeholder(&serde_json::Value::Object(
                            obj.clone(),
                        ))
                    }) {
                        Some(format!(
                            "{}::from_json(str_replace('{}', ${base_var}, '{}'))",
                            elem_type,
                            crate::e2e::codegen::MOCK_URL_PLACEHOLDER,
                            php_literal
                        ))
                    } else {
                        Some(format!("{}::from_json('{}')", elem_type, php_literal))
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

/// Filters out empty string enum values from JSON objects before rendering.
///
/// When a field has an empty string value, it's treated as a missing/null enum field
/// and should not be included in the PHP array.
pub(super) fn filter_empty_enum_strings(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter_map(|(k, v)| {
                    // Skip empty string values (typically represent missing enum variants)
                    if let serde_json::Value::String(s) = v {
                        if s.is_empty() {
                            return None;
                        }
                    }
                    // Recursively filter nested objects and arrays
                    Some((k.clone(), filter_empty_enum_strings(v)))
                })
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let filtered: Vec<serde_json::Value> = arr.iter().map(filter_empty_enum_strings).collect();
            serde_json::Value::Array(filtered)
        }
        other => other.clone(),
    }
}

/// Convert a `serde_json::Value` to a PHP literal string.
pub(super) fn json_to_php(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_php(s)),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_php).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\" => {}", escape_php(k), json_to_php(v)))
                .collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Get the field type name for a given struct and field name.
///
/// Returns the string name of the field's type if it's a Named type, otherwise None.
pub(super) fn get_field_type_name(
    struct_name: &str,
    field_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    type_defs
        .iter()
        .find(|td| td.name == struct_name)
        .and_then(|td| td.fields.iter().find(|f| f.name == field_name))
        .and_then(|field| match &field.ty {
            TypeRef::Named(name) => Some(name.clone()),
            TypeRef::Optional(inner) => match &**inner {
                TypeRef::Named(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        })
}

/// Like `json_to_php` but optionally converts object keys to lowerCamelCase.
///
/// When `serde_rename_all` is Some("camelCase"), recursively converts all object keys
/// from snake_case to camelCase. Otherwise, passes keys through unchanged.
///
/// Uses IR type information to determine the correct serde_rename_all setting for
/// nested structs — each nested object's keys are transformed based on whether that
/// specific struct type has `#[serde(rename_all = "camelCase")]`, not inherited from
/// the parent.
///
/// Used when generating PHP option arrays passed to `from_json()` — PHP binding
/// structs respect the serde attributes of the underlying Rust core types, so we only
/// apply camelCase transformation when the target type explicitly declares it.
pub(super) fn json_to_php_camel_keys_with_types(
    value: &serde_json::Value,
    current_type_name: Option<&str>,
    serde_rename_all: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let final_key = if serde_rename_all == Some("camelCase") {
                        k.to_lower_camel_case()
                    } else {
                        k.to_string()
                    };
                    // When recursing into a nested object, propagate the parent's
                    // serde_rename_all. For PHP this matters because all binding structs are
                    // emitted with the same `#[serde(rename_all = "...")]` setting (driven by
                    // the language-effective rename strategy), so nested objects use the same
                    // strategy as the parent. The Rust core type's serde_rename_all on the
                    // nested field's type is irrelevant — the binding deserializer reads the
                    // binding struct's attributes.
                    let nested_type_name = current_type_name.and_then(|tn| get_field_type_name(tn, k, type_defs));
                    format!(
                        "\"{}\" => {}",
                        escape_php(&final_key),
                        json_to_php_camel_keys_with_types(v, nested_type_name.as_deref(), serde_rename_all, type_defs)
                    )
                })
                .collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|item| json_to_php_camel_keys_with_types(item, current_type_name, serde_rename_all, type_defs))
                .collect();
            format!("[{}]", items.join(", "))
        }
        _ => json_to_php(value),
    }
}

/// Returns true if the type name is a PHP reserved/primitive type that cannot be imported.
pub(super) fn is_php_reserved_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "string"
            | "int"
            | "integer"
            | "float"
            | "double"
            | "bool"
            | "boolean"
            | "array"
            | "object"
            | "null"
            | "void"
            | "callable"
            | "iterable"
            | "never"
            | "self"
            | "parent"
            | "static"
            | "true"
            | "false"
            | "mixed"
    )
}
