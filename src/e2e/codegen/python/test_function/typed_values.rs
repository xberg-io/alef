//! Typed Python value rendering for generated test functions.

use std::collections::HashMap;

use heck::ToSnakeCase;

use crate::e2e::escape::escape_python;

use super::super::json::json_to_python_literal;

/// Resolve the enum type name for a field if it's an enum type in the TypeDef,
/// and return None if it's not an enum or the type cannot be resolved.
pub(in crate::e2e::codegen::python) fn resolve_field_enum_type(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Option<String> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    // Unwrap Optional and Vec wrappers to get the inner type
    let inner_name = match &field.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }?;

    // Check if this is an enum type
    if enums.iter().any(|e| e.name == inner_name) {
        Some(inner_name.to_string())
    } else {
        None
    }
}

/// Returns `true` if the arg was fully emitted (caller should `continue`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_json_object_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    element_type: &Option<String>,
    fixture_id: &str,
    has_host_root_route: bool,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> bool {
    if crate::e2e::codegen::value_contains_mock_url_placeholder(value) {
        return emit_json_object_arg_with_mock_url(
            arg_bindings,
            kwarg_exprs,
            value,
            var_name,
            options_type,
            options_via,
            fixture_id,
            has_host_root_route,
        );
    }

    match options_via {
        "dict" => {
            // When we have an array of objects and an element_type, emit dict literals (not constructor calls).
            // The bindings expect [{"type": "click", "selector": "#id"}, ...], not [PageAction(...), ...]
            if let (Some(_elem_type), Some(arr)) = (element_type, value.as_array()) {
                if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
                    let items: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_object())
                        .map(|obj| {
                            let dict_items: Vec<String> = obj
                                .iter()
                                .map(|(k, v)| {
                                    format!(
                                        "{}: {}",
                                        json_to_python_literal(&serde_json::Value::String(k.clone())),
                                        json_to_python_literal(v)
                                    )
                                })
                                .collect();
                            format!("{{{}}}", dict_items.join(", "))
                        })
                        .collect();
                    arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                    kwarg_exprs.push(var_name.to_string());
                    return true;
                }
            }
            // Fall through to default dict behavior
            let literal = json_to_python_literal(value);
            let noqa = if literal.contains("/tmp/") {
                "  # noqa: S108"
            } else {
                ""
            };
            arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "json" => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let escaped = escape_python(&json_str);
            arg_bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "from_json" => {
            if let Some(opts_type) = options_type {
                let json_str = serde_json::to_string(value).unwrap_or_default();
                let escaped = escape_python(&json_str);
                arg_bindings.push(format!("    {var_name} = {opts_type}.from_json(\"{escaped}\")"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
        _ => {
            // When we have an array with element_type, construct typed instances for Python.
            if let Some(elem_type) = element_type {
                if !value.is_null() {
                    if let Some(arr) = value.as_array() {
                        if arr.iter().all(|item| item.is_object()) {
                            let items: Vec<String> = arr
                                .iter()
                                .filter_map(|item| item.as_object())
                                .map(|obj| emit_python_typed_instance(obj, elem_type))
                                .collect();
                            arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                            kwarg_exprs.push(var_name.to_string());
                            return true;
                        }
                    }
                }
            }
            // "kwargs" mode
            if let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) {
                let kwargs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let snake_key = k.to_snake_case();
                        let py_val = if let Some(enum_type) = enum_fields.get(k) {
                            // Explicit override: use the configured enum type
                            if let Some(s) = v.as_str() {
                                format!("{enum_type}(\"{s}\")")
                            } else {
                                json_to_python_literal(v)
                            }
                        } else if let Some(auto_enum_type) =
                            resolve_field_enum_type(k, Some(opts_type), type_defs, enums)
                        {
                            // Auto-detect: if field type is an enum, emit as EnumType("variant").
                            // Constructor-call form works for both (str, Enum) subclasses (where
                            // lookup-by-value resolves to the canonical variant) and #[pyclass]
                            // tagged-union structs (which expose a serde-backed constructor).
                            // Attribute access (EnumType.VARIANT) fails for pyclass-emitted
                            // enums because they have no class-level variant constants.
                            if let Some(s) = v.as_str() {
                                format!("{auto_enum_type}(\"{s}\")")
                            } else {
                                json_to_python_literal(v)
                            }
                        } else {
                            json_to_python_literal(v)
                        };
                        format!("{snake_key}={py_val}")
                    })
                    .collect();
                let constructor = format!("{opts_type}({})", kwargs.join(", "));
                arg_bindings.push(format!("    {var_name} = {constructor}"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_json_object_arg_with_mock_url(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    fixture_id: &str,
    has_host_root_route: bool,
) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
    let fallback = format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'");
    let base_expr = if has_host_root_route {
        format!("os.environ.get('{env_key}') or {fallback}")
    } else {
        fallback
    };
    arg_bindings.push(format!("    {var_name}_mock_base_url = {base_expr}"));
    arg_bindings.push(format!(
        "    {var_name}_json = \"{escaped}\".replace(\"{}\", {var_name}_mock_base_url)",
        crate::e2e::codegen::MOCK_URL_PLACEHOLDER
    ));

    match (options_via, options_type) {
        ("from_json", Some(opts_type)) => {
            arg_bindings.push(format!("    {var_name} = {opts_type}.from_json({var_name}_json)"));
        }
        ("dict", _) | (_, None) | ("json", _) => {
            arg_bindings.push(format!("    {var_name} = json.loads({var_name}_json)"));
        }
        (_, Some(opts_type)) => {
            arg_bindings.push(format!("    {var_name} = {opts_type}(**json.loads({var_name}_json))"));
        }
    }
    kwarg_exprs.push(var_name.to_string());
    true
}

pub(super) fn emit_bytes_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
) {
    if let Some(raw) = value.as_str() {
        match super::super::helpers::classify_bytes_value(raw) {
            super::super::helpers::BytesKind::FilePath => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = Path(\"{escaped}\").read_bytes()"));
            }
            super::super::helpers::BytesKind::InlineText => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = b\"{escaped}\""));
            }
            super::super::helpers::BytesKind::Base64 => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = base64.b64decode(\"{escaped}\")"));
            }
        }
    } else {
        arg_bindings.push(format!("    {var_name} = None"));
    }
    kwarg_exprs.push(var_name.to_string());
}

/// Emit a Python dict literal for a typed object-array element.
#[allow(dead_code)]
fn emit_python_object_item(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let items: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            format!(
                "{}: {}",
                json_to_python_literal(&serde_json::Value::String(k.clone())),
                json_to_python_literal(v)
            )
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// Emit a Python constructor call for a typed instance (e.g., BatchFileItem(...)).
fn emit_python_typed_instance(obj: &serde_json::Map<String, serde_json::Value>, elem_type: &str) -> String {
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            format!("{}={}", snake_key, json_to_python_literal(v))
        })
        .collect();
    format!("{}({})", elem_type, kwargs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_bytes_arg_file_path_uses_path_read_bytes() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("pdf/memo.pdf".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "content");
        assert!(bindings[0].contains("Path("), "got: {:?}", bindings[0]);
        assert!(bindings[0].contains("read_bytes"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_bytes_arg_base64_uses_b64decode() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("/9j/4AAQ".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "data");
        assert!(bindings[0].contains("b64decode"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_json_object_arg_enum_field_emits_constructor_call() {
        use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

        let enum_def = EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "demo::OutputFormat".to_string(),
            variants: vec![EnumVariant {
                name: "Markdown".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let type_def = TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "output_format".to_string(),
                ty: TypeRef::Named("OutputFormat".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let enums = vec![enum_def];
        let type_defs = vec![type_def];

        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::json!({"output_format": "markdown"});
        let done = emit_json_object_arg(
            &mut bindings,
            &mut exprs,
            &value,
            "opts",
            Some("ExtractionConfig"),
            "kwargs",
            &HashMap::new(),
            &None,
            "fixture",
            false,
            &type_defs,
            &enums,
        );
        assert!(done);
        // Constructor-call form works for both (str, Enum) subclasses and #[pyclass] tagged-union
        // structs. Attribute access (OutputFormat.MARKDOWN) fails for the latter because they have
        // no class-level variant constants.
        assert!(
            bindings[0].contains("OutputFormat(\"markdown\")"),
            "expected constructor-call emission, got: {:?}",
            bindings[0]
        );
        assert!(
            !bindings[0].contains("OutputFormat.MARKDOWN"),
            "must not emit attribute access, got: {:?}",
            bindings[0]
        );
    }

    #[test]
    fn emit_json_object_arg_dict_mode_emits_literal() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::json!({"key": "val"});
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let done = emit_json_object_arg(
            &mut bindings,
            &mut exprs,
            &value,
            "opts",
            None,
            "dict",
            &HashMap::new(),
            &None,
            "fixture",
            false,
            &type_defs,
            &enums,
        );
        assert!(done);
        assert!(bindings[0].contains("\"key\""), "got: {:?}", bindings[0]);
    }

    #[test]
    fn resolve_field_enum_type_detects_enum_field() {
        use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

        let enum_def = EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "module::TierStrategy".to_string(),
            variants: vec![EnumVariant {
                name: "Auto".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let type_def = TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "module::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "tier_strategy".to_string(),
                ty: TypeRef::Named("TierStrategy".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let enums = vec![enum_def];
        let type_defs = vec![type_def];

        let result = resolve_field_enum_type("tier_strategy", Some("ConversionOptions"), &type_defs, &enums);
        assert_eq!(result, Some("TierStrategy".to_string()));
    }

    #[test]
    fn resolve_field_enum_type_returns_none_for_non_enum_field() {
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};

        let type_def = TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "module::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Named("u64".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let enums: Vec<crate::core::ir::EnumDef> = vec![];
        let type_defs = vec![type_def];

        let result = resolve_field_enum_type("timeout", Some("ConversionOptions"), &type_defs, &enums);
        assert_eq!(result, None);
    }
}
