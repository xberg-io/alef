//! PHP e2e PHP-literal rendering helpers.

use crate::core::ir::TypeRef;
use crate::e2e::escape::escape_php;
use heck::ToLowerCamelCase;

/// Render a fixture object as a PHP named-argument constructor call for `type_name`.
///
/// `pub(crate)` so the PHP binding backend's own tests can compare this literal against the
/// `#[php(constructor)]` signature it emits for the same `TypeDef` -- see
/// `backends::php::gen_bindings::types::structs::tests`. ~keep
pub(crate) fn render_native_php_dto(
    namespace: &str,
    type_name: &str,
    value: &serde_json::Value,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> Option<String> {
    render_native_php_dto_at(namespace, type_name, value, type_defs, files, "")
}

fn render_native_php_dto_at(
    namespace: &str,
    type_name: &str,
    value: &serde_json::Value,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
) -> Option<String> {
    let object = value.as_object()?;
    let type_def = type_defs.iter().find(|candidate| candidate.name == type_name)?;
    let constructor_fields = type_def
        .fields
        .iter()
        .filter(|field| !field.binding_excluded && field.cfg.is_none())
        .collect::<Vec<_>>();
    if !constructor_fields
        .iter()
        .any(|field| !field.optional && php_native_constructor_type(&field.ty))
        || constructor_fields
            .iter()
            .any(|field| object.contains_key(&field.name) && !php_native_constructor_type(&field.ty))
        || constructor_fields.iter().any(|field| {
            !field.optional && !matches!(field.ty, TypeRef::Optional(_)) && !object.contains_key(&field.name)
        })
    {
        return None;
    }
    let fields = constructor_fields
        .into_iter()
        .filter_map(|field| object.get(&field.name).map(|value| (field, value)))
        .map(|(field, value)| {
            // The named argument must be the PHP binding's CONSTRUCTOR PARAMETER name, which
            // `backends::php` builds with `naming::to_php_name` (see
            // `types::structs::gen_struct_methods_impl`, `helpers::params`). `public_field_name`
            // is the same lower-camel transform PLUS keyword escaping, so the two agreed on every
            // field except a PHP-keyword-named one: the binding declares `$list`, this emitted
            // `list_:`, and PHP rejects the call with `Unknown named parameter $list_`. PHP allows
            // a reserved word as a parameter name and as a named argument, so the escape is not
            // just a second spelling -- it is the wrong one. ~keep
            let name = crate::codegen::naming::to_php_name(&field.name);
            let field_pointer = format!("{pointer}/{}", field.name);
            let value = render_native_php_value(namespace, value, &field.ty, type_defs, files, &field_pointer)?;
            Some(minijinja::context! { name => name, value => value })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(
        crate::e2e::template_env::render(
            "php/typed_dto.jinja",
            minijinja::context! { namespace => namespace, type_name => type_name, fields => fields },
        )
        .trim_end()
        .to_string(),
    )
}

fn php_native_constructor_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Bytes => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => php_native_constructor_type(inner),
        _ => false,
    }
}

fn render_native_php_value(
    namespace: &str,
    value: &serde_json::Value,
    ty: &TypeRef,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
) -> Option<String> {
    if files.iter().any(|file| file.field == pointer) && matches!(ty, TypeRef::Bytes) {
        return Some(format!("file_get_contents(\"{}\")", escape_php(value.as_str()?)));
    }
    match (value, ty) {
        (serde_json::Value::Null, _) => Some("null".into()),
        (value, TypeRef::Optional(inner)) => {
            render_native_php_value(namespace, value, inner, type_defs, files, pointer)
        }
        (value, TypeRef::Named(name)) => render_native_php_dto_at(namespace, name, value, type_defs, files, pointer),
        (serde_json::Value::Array(values), TypeRef::Vec(inner)) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                render_native_php_value(namespace, value, inner, type_defs, files, &format!("{pointer}/{index}"))
            })
            .collect::<Option<Vec<_>>>()
            .map(|items| format!("[{}]", items.join(", "))),
        (serde_json::Value::String(value), TypeRef::String | TypeRef::Char | TypeRef::Path) => {
            Some(format!("\"{}\"", escape_php(value)))
        }
        (serde_json::Value::Bool(value), TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) => {
            Some(value.to_string())
        }
        (serde_json::Value::Number(value), TypeRef::Primitive(_)) => Some(value.to_string()),
        _ => None,
    }
}

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

/// Find the `FieldDef` on `struct_name` that fixture key `key` refers to, matching either the
/// field's Rust name or its wire name (`#[serde(rename = ...)]` / a container `rename_all`) —
/// a fixture may key a field either way. Without the wire-name arm, a field whose wire name
/// diverges from its Rust name (e.g. `ChatCompletionTool.tool_type` renamed to wire `type`) is
/// invisible to every lookup here even though the fixture key is exactly what that field
/// declares, since only `field.name` was ever compared. ~keep
fn resolve_field<'a>(
    struct_name: &str,
    key: &str,
    type_defs: &'a [crate::core::ir::TypeDef],
) -> Option<&'a crate::core::ir::FieldDef> {
    let definition = type_defs.iter().find(|td| td.name == struct_name)?;
    definition.fields.iter().find(|field| {
        field.name == key
            || crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                definition.serde_rename_all.as_deref(),
            ) == key
    })
}

/// True when `field_name` on `struct_name` is a plain `String` or `Optional<String>`
/// field — i.e. a field where an empty string `""` is a meaningful value the fixture
/// intends to send (e.g. `ExtractInput.mime_type = ""` to exercise the empty-MIME
/// error path), not an absent enum variant. Returns false when the struct or field is
/// unknown, so callers fall back to the conservative "drop empty string" behaviour.
pub(super) fn field_is_string_typed(
    struct_name: Option<&str>,
    field_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> bool {
    let Some(struct_name) = struct_name else {
        return false;
    };
    resolve_field(struct_name, field_name, type_defs)
        .map(|field| match &field.ty {
            TypeRef::String => true,
            TypeRef::Optional(inner) => matches!(**inner, TypeRef::String),
            _ => false,
        })
        .unwrap_or(false)
}

/// Drops empty string values only
/// for fields that are NOT plain `String`/`Optional<String>` (i.e. enum fields, where
/// `""` represents an absent variant and would fail deserialization), while preserving
/// `""` for genuine string fields whose emptiness is meaningful (e.g. an explicit empty
/// `mime_type` that must trigger the core's empty-MIME error). `current_type_name` is the
/// binding struct the object deserialises into; nested objects resolve their own type.
pub(super) fn filter_empty_enum_strings_with_types(
    value: &serde_json::Value,
    current_type_name: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter_map(|(k, v)| {
                    if let serde_json::Value::String(s) = v
                        && s.is_empty()
                        && !field_is_string_typed(current_type_name, k, type_defs)
                    {
                        return None;
                    }
                    let nested_type_name = current_type_name.and_then(|tn| get_field_type_name(tn, k, type_defs));
                    Some((
                        k.clone(),
                        filter_empty_enum_strings_with_types(v, nested_type_name.as_deref(), type_defs),
                    ))
                })
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| filter_empty_enum_strings_with_types(v, None, type_defs))
                .collect(),
        ),
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
/// Returns the string name of the field's type if it names one through any number of
/// `Option`/`Vec` wrappers (see [`crate::e2e::codegen::call_ir::named_type`]), otherwise None.
/// Unwrapping `Vec` matters here as much as `Option` does: a `Vec<ChatCompletionTool>` field
/// (e.g. `ChatCompletionRequest.tools`) used to resolve to `None`, so every element of the
/// array below recursed with `current_type_name` reset to `None` — losing the owner type for
/// exactly the array-of-struct shape a tool-calling request is built from, and silently
/// re-enabling the blind per-key camelCase this module exists to avoid one level down. ~keep
pub(super) fn get_field_type_name(
    struct_name: &str,
    field_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    resolve_field(struct_name, field_name, type_defs)
        .and_then(|field| crate::e2e::codegen::call_ir::named_type(&field.ty))
        .map(str::to_string)
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
                    // `k` is the fixture's WIRE key (e.g. `type` for `ChatCompletionTool` from
                    // the core Rust struct's own `#[serde(rename = "type")]`), but the PHP
                    // binding's host property/alias is always spelled off the RUST FIELD name
                    // (`tool_type` -> `toolType`), never off that wire name -- see
                    // `codegen::naming::wire`'s module doc. Blindly camelCasing `k` itself
                    // produced `"type"` (`to_lower_camel_case("type") == "type"`) against a
                    // binding that only accepts `tool_type`/`toolType`, silently leaving the
                    // field at its `#[serde(default)]` value. Resolving through the field
                    // first, and camelCasing ITS name, fixes both the unrenamed case (already
                    // identical) and the renamed one. A key IR cannot resolve (an opaque/
                    // untyped object, or `current_type_name` unset) falls back to the previous
                    // blind conversion. ~keep
                    let final_key = if serde_rename_all == Some("camelCase") {
                        match current_type_name.and_then(|tn| resolve_field(tn, k, type_defs)) {
                            Some(field) => crate::codegen::naming::to_php_name(&field.name),
                            None => k.to_lower_camel_case(),
                        }
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

#[cfg(test)]
mod native_dto_tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef};

    #[test]
    fn renders_known_struct_as_native_php_constructor() {
        let type_defs = [TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "display_name".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let rendered = render_native_php_dto(
            "Sample",
            "SampleRequest",
            &serde_json::json!({"display_name": "Ada"}),
            &type_defs,
            &[],
        );

        assert_eq!(
            rendered.as_deref(),
            Some("new \\Sample\\SampleRequest(displayName: \"Ada\")")
        );
    }

    #[test]
    fn renders_file_pointer_as_binary_string_read() {
        let type_defs = [TypeDef {
            name: "Upload".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::Bytes,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let files = [crate::e2e::fixture::FixtureDocsFileInput {
            field: "/content".into(),
            path: "guide.pdf".into(),
        }];
        let rendered = render_native_php_dto(
            "Sample",
            "Upload",
            &serde_json::json!({"content": "guide.pdf"}),
            &type_defs,
            &files,
        )
        .expect("native DTO");
        assert!(
            rendered.contains("content: file_get_contents(\"guide.pdf\")"),
            "{rendered}"
        );
    }
}

#[cfg(test)]
mod wire_name_tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef};

    /// A field keyed by its WIRE name (`#[serde(rename = "type")]`, e.g.
    /// `ChatCompletionTool.tool_type`) must resolve to the PHP binding's property name
    /// (`toolType`, off the Rust field), not a blind camelCase of the wire key itself
    /// (`type`). Regression for the liter-llm `tool_calling` e2e defect. ~keep
    #[test]
    fn wire_renamed_field_resolves_to_the_php_property_name() {
        let type_defs = [TypeDef {
            name: "ChatCompletionTool".into(),
            fields: vec![FieldDef {
                name: "tool_type".into(),
                ty: TypeRef::String,
                serde_rename: Some("type".into()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];

        let rendered = json_to_php_camel_keys_with_types(
            &serde_json::json!({"type": "function"}),
            Some("ChatCompletionTool"),
            Some("camelCase"),
            &type_defs,
        );

        assert_eq!(rendered, "[\"toolType\" => \"function\"]");
    }

    /// A `Vec<Named>` field (e.g. `ChatCompletionRequest.tools: Vec<ChatCompletionTool>`) must
    /// propagate its unwrapped element type to array elements, not just an `Option<Named>`
    /// field -- otherwise every element recurses with no owner type and the wire-renamed field
    /// above is unreachable from a real request body.
    #[test]
    fn array_of_struct_field_propagates_element_type_to_items() {
        let type_defs = [
            TypeDef {
                name: "ChatCompletionRequest".into(),
                fields: vec![FieldDef {
                    name: "tools".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("ChatCompletionTool".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ChatCompletionTool".into(),
                fields: vec![FieldDef {
                    name: "tool_type".into(),
                    ty: TypeRef::String,
                    serde_rename: Some("type".into()),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ];

        let rendered = json_to_php_camel_keys_with_types(
            &serde_json::json!({"tools": [{"type": "function"}]}),
            Some("ChatCompletionRequest"),
            Some("camelCase"),
            &type_defs,
        );

        assert_eq!(rendered, "[\"tools\" => [[\"toolType\" => \"function\"]]]");
    }
}
