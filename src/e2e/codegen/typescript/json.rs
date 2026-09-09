//! JSON-to-JavaScript literal conversion utilities.

use crate::codegen::naming::underscore_camel_case;
use crate::e2e::escape::{escape_js, expand_fixture_templates};

/// Convert a `serde_json::Value` to a JavaScript literal string.
pub(super) fn json_to_js(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            let expanded = expand_fixture_templates(s);
            format!("\"{}\"", escape_js(&expanded))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            // For integers outside JS safe range, emit as string to avoid precision loss.
            if let Some(i) = n.as_i64()
                && !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&i)
            {
                return format!("Number(\"{i}\")");
            }
            if let Some(u) = n.as_u64()
                && u > 9_007_199_254_740_991
            {
                return format!("Number(\"{u}\")");
            }
            n.to_string()
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = js_object_key(k);
                    format!("{key}: {}", json_to_js(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}

/// Convert a `serde_json::Value` to an indented multi-line JavaScript literal.
///
/// Top-level objects are always expanded to multi-line form with trailing commas
/// so that formatters (e.g. oxfmt) leave the output unchanged. Scalar values and
/// arrays are emitted inline. Nested objects are also expanded to multi-line.
///
/// The `indent` parameter controls the base indentation in spaces for all but
/// the outermost `{`/`}`. Pass 4 for a top-level `expect(data).toEqual({...})`
/// inside a two-space-indented test body.
pub(super) fn json_to_js_multiline(value: &serde_json::Value, indent: usize) -> String {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let pad = " ".repeat(indent);
            let inner_pad = " ".repeat(indent + 2);
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = js_object_key(k);
                    format!("{inner_pad}{key}: {},", json_to_js_multiline(v, indent + 2))
                })
                .collect();
            format!("{{\n{}\n{pad}}}", entries.join("\n"))
        }
        // Non-object values are emitted inline.
        other => json_to_js(other),
    }
}

/// Render `key` as an object-literal key, quoting it when it is not a bare JS identifier
/// (hyphens, spaces, a leading digit).
/// Computed `__proto__` remains an own data property instead of changing the prototype. ~keep
pub(super) fn js_object_key(key: &str) -> String {
    if key == "__proto__" {
        return "[\"__proto__\"]".to_string();
    }
    if !key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !key.starts_with(|c: char| c.is_ascii_digit())
    {
        key.to_string()
    } else {
        format!("\"{}\"", escape_js(key))
    }
}

/// Convert a `serde_json::Value` to a JavaScript literal string with camelCase object keys.
///
/// NAPI-RS bindings use camelCase for JavaScript field names. This variant converts
/// snake_case object keys (as written in fixture JSON) to camelCase so that the
/// generated config objects match the NAPI binding's expected field names.
pub(super) fn json_to_js_camel(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = js_object_key(&underscore_camel_case(k));
                    format!("{key}: {}", json_to_js_camel(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js_camel).collect();
            format!("[{}]", items.join(", "))
        }
        // Scalars and null delegate to the standard converter.
        other => json_to_js(other),
    }
}

/// Find the `FieldDef` on `owner_type` that fixture key `key` refers to, matching either the
/// field's Rust name or its wire name (`#[serde(rename = ...)]` / a container `rename_all`) --
/// mirrors `typescript::test_file::builders::resolve_owner_field` and PHP's own equivalent
/// (`php::values::resolve_field`), all three needing the same fixture-key -> `FieldDef` reverse
/// lookup for the same reason.
fn resolve_owner_field<'a>(
    owner_type: Option<&'a crate::core::ir::TypeDef>,
    key: &str,
) -> Option<&'a crate::core::ir::FieldDef> {
    let definition = owner_type?;
    definition.fields.iter().find(|field| {
        field.name == key
            || crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                definition.serde_rename_all.as_deref(),
            ) == key
    })
}

/// Like [`json_to_js_camel`], but resolves each object key through the core IR when the
/// current object's owner type is known, rather than blindly camelCasing the fixture's wire
/// key.
///
/// NAPI's `#[napi(object)]` derive names a JS field from the RUST FIELD, never from a
/// `#[serde(rename = ...)]` on that field (its `FromNapiValue` impl does not consult serde at
/// all -- see `codegen::naming::wire`'s module doc). `json_to_js_camel` camelCases the fixture's
/// WIRE key, which is only ever the same string when a field is not serde-renamed
/// (`max_tokens` -> `maxTokens` either way). A field like `ChatCompletionTool.tool_type`
/// (`#[serde(rename = "type")]`) diverges: the fixture's wire key is `type`, camelCasing it
/// stays `type`, and the binding only accepts `toolType`, silently leaving the field at its
/// `#[serde(default)]` value.
///
/// Struct keys resolve through their declared fields; explicit Map and Json payloads retain
/// their data keys verbatim. Container value types propagate recursively so a map's entry keys
/// remain unchanged while fields inside its typed struct values still resolve correctly.
/// Unknown struct types and fields retain the legacy camelCase fallback. Values remain literals
/// rather than synthesized enum members: fixtures may contain free-form union payloads or
/// intentionally invalid enum values that must reach the binding unchanged. ~keep
pub(super) fn json_to_js_camel_with_types(
    value: &serde_json::Value,
    current_type_name: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let field_type = current_type_name.map(|name| crate::core::ir::TypeRef::Named(name.to_string()));
    json_to_js_with_type(value, field_type.as_ref(), type_defs)
}

fn json_to_js_with_type(
    value: &serde_json::Value,
    field_type: Option<&crate::core::ir::TypeRef>,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    use crate::core::ir::TypeRef;
    if let Some(TypeRef::Optional(inner)) = field_type {
        return json_to_js_with_type(value, Some(inner), type_defs);
    }
    let owner = match field_type {
        Some(TypeRef::Named(name)) => type_defs.iter().find(|definition| definition.name == *name),
        _ => None,
    };
    match (value, field_type) {
        (serde_json::Value::Object(map), Some(TypeRef::Map(_, inner))) => {
            let entries = map
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}: {}",
                        js_object_key(key),
                        json_to_js_with_type(value, Some(inner), type_defs)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{ {} }}", entries.join(", "))
        }
        (serde_json::Value::Object(map), _) if owner.is_some() => {
            let entries = map
                .iter()
                .map(|(key, value)| {
                    let field = resolve_owner_field(owner, key);
                    let key = field.map_or_else(
                        || underscore_camel_case(key),
                        |field| crate::codegen::naming::to_node_name(&field.name),
                    );
                    format!(
                        "{}: {}",
                        js_object_key(&key),
                        json_to_js_with_type(value, field.map(|field| &field.ty), type_defs)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{ {} }}", entries.join(", "))
        }
        (serde_json::Value::Array(items), Some(TypeRef::Vec(inner))) => {
            let items = items
                .iter()
                .map(|value| json_to_js_with_type(value, Some(inner), type_defs))
                .collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        }
        (serde_json::Value::Array(items), Some(TypeRef::Named(_))) => {
            let items = items
                .iter()
                .map(|value| json_to_js_with_type(value, field_type, type_defs))
                .collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        }
        (_, Some(TypeRef::Json)) => json_to_js(value),
        _ => json_to_js_camel(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_js_string_escapes_double_quotes() {
        let val = serde_json::Value::String("say \"hello\"".to_string());
        let out = json_to_js(&val);
        assert!(out.contains("\\\""), "got: {out}");
    }

    #[test]
    fn json_to_js_null_returns_null_literal() {
        assert_eq!(json_to_js(&serde_json::Value::Null), "null");
    }

    #[test]
    fn json_to_js_camel_converts_object_keys() {
        let val = serde_json::json!({ "my_field": 1 });
        let out = json_to_js_camel(&val);
        assert!(out.contains("myField"), "got: {out}");
        assert!(!out.contains("my_field"), "got: {out}");
    }

    /// A field keyed by its WIRE name (`#[serde(rename = "type")]`, e.g.
    /// `ChatCompletionTool.tool_type`) must resolve to the napi binding's JS field name
    /// (`toolType`, off the Rust field), not a blind camelCase of the wire key itself
    /// (`type`, which is already single-word and would not change). Regression for the
    /// liter-llm `tool_calling` e2e defect. ~keep
    #[test]
    fn wire_renamed_field_resolves_to_the_node_property_name() {
        let type_defs = [crate::core::ir::TypeDef {
            name: "ChatCompletionTool".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "tool_type".into(),
                ty: crate::core::ir::TypeRef::String,
                serde_rename: Some("type".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];

        let out = json_to_js_camel_with_types(
            &serde_json::json!({"type": "function"}),
            Some("ChatCompletionTool"),
            &type_defs,
        );

        assert_eq!(out, "{ toolType: \"function\" }");
    }

    /// A `Vec<Named>` field (e.g. `ChatCompletionRequest.tools: Vec<ChatCompletionTool>`) must
    /// propagate its unwrapped element type to array elements, otherwise every element
    /// recurses with no owner type and the wire-renamed field above is unreachable from a real
    /// request body.
    #[test]
    fn array_of_struct_field_propagates_element_type_to_items() {
        let type_defs = [
            crate::core::ir::TypeDef {
                name: "ChatCompletionRequest".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "tools".into(),
                    ty: crate::core::ir::TypeRef::Vec(Box::new(crate::core::ir::TypeRef::Named(
                        "ChatCompletionTool".into(),
                    ))),
                    ..Default::default()
                }],
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "ChatCompletionTool".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "tool_type".into(),
                    ty: crate::core::ir::TypeRef::String,
                    serde_rename: Some("type".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];

        let out = json_to_js_camel_with_types(
            &serde_json::json!({"tools": [{"type": "function"}]}),
            Some("ChatCompletionRequest"),
            &type_defs,
        );

        assert_eq!(out, "{ tools: [{ toolType: \"function\" }] }");
    }
}
