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
                    // Quote keys that aren't valid JS identifiers (contain hyphens, spaces, etc.)
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
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
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
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
pub(super) fn js_object_key(key: &str) -> String {
    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
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
/// Deliberately narrow in what it changes: only the KEY is resolved through the IR; every VALUE
/// is still rendered by the plain, non-type-aware converters (`json_to_js`/`json_to_js_camel`
/// for values whose own nested owner type could not be resolved). This module intentionally does
/// not reach for `ts_builder_expression`'s enum-literal synthesis
/// (`declared_enum_member_for_prefixed`/`node_tagged_unit_variant_literal`) here: that machinery
/// assumes an enum-typed field's fixture value is always one of the enum's declared variants,
/// which does not hold for an untagged string/composite union modeled as an IR enum over
/// arbitrary free text (e.g. `ModerationRequest.input: ModerationInput`) or for a fixture that
/// deliberately sends an undeclared value to exercise an error path (e.g.
/// `CreateFileRequest.purpose: "invalid-purpose"`) -- routing either through that synthesis
/// manufactures a nonexistent enum member reference (`ModerationInput.` for an empty string,
/// `FilePurpose.InvalidPurpose` for a value with no matching variant), a hard compile error in
/// the first case and a silently wrong test in the second. Every host binding's `ts_type`
/// override for an enum-typed field already accepts the plain wire string as an alternative
/// (`ts_type = "ToolType | 'function'"`), so leaving values as plain JS literals is not a
/// downgrade -- `toolType: "function"` type-checks exactly as `toolType: ToolType.Function`
/// does. ~keep
pub(super) fn json_to_js_camel_with_types(
    value: &serde_json::Value,
    current_type_name: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let owner_type = current_type_name.and_then(|name| type_defs.iter().find(|definition| definition.name == name));
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let (key, nested_type_name) = match resolve_owner_field(owner_type, k) {
                        Some(field) => (
                            js_object_key(&crate::codegen::naming::to_node_name(&field.name)),
                            crate::e2e::codegen::call_ir::named_type(&field.ty).map(str::to_string),
                        ),
                        None => (js_object_key(&underscore_camel_case(k)), None),
                    };
                    format!(
                        "{key}: {}",
                        json_to_js_camel_with_types(v, nested_type_name.as_deref(), type_defs)
                    )
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        serde_json::Value::Array(arr) => {
            // Array elements share the CONTAINER field's already-unwrapped `current_type_name`
            // (`named_type` unwraps `Vec` alongside `Option`), not a per-index lookup -- there is
            // no key to resolve a field through at this level, only the element type carried
            // down from the field that held this array.
            let items: Vec<String> = arr
                .iter()
                .map(|item| json_to_js_camel_with_types(item, current_type_name, type_defs))
                .collect();
            format!("[{}]", items.join(", "))
        }
        other => json_to_js(other),
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
