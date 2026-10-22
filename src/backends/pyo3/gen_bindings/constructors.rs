use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Resolve the PyO3 constructor parameter identifier for a field — the single source of truth for
/// the `#[new]` signature AND the `.pyi` `__init__` stub, so the two cannot drift apart.
///
/// Prefers the serde rename (the wire name, deliberately used for constructor params so the public
/// surface matches the other language bindings — see [`replace_constructor_with_serde_rename`]),
/// then a per-language `rename_fields` entry, then the bare field name. Only applies a resolved name
/// when it is a syntactically valid Rust identifier (e.g. `"self-harm"` falls back to the field
/// name); a valid name that is also a Rust keyword (e.g. `"type"`) is escaped as `r#type`.
///
/// The resolved name is run through `python_ident` before the Rust escape because this identifier
/// is not merely a Rust parameter — PyO3 publishes it verbatim as the keyword-argument name, and
/// `r#` is stripped when it does. A wire name that is a Python keyword (`global`, either as the
/// field name or as an explicit `#[serde(rename)]`) therefore reaches Python as `Type(global=...)`,
/// which no amount of Rust-side escaping can make parse. Python has no escape mechanism, so the
/// rename is forced; `python_ident` is the same helper the `.pyi` stubs and the dataclass/TypedDict
/// field emitters already use, so escaping here converges those surfaces rather than adding a
/// fourth spelling. The JSON wire name is unaffected — serde attributes carry it independently. ~keep
pub(in crate::backends::pyo3) fn resolve_param_ident<'a>(
    field_name: &'a str,
    serde_rename: Option<&'a String>,
    config_renames: Option<&HashMap<String, String>>,
) -> String {
    use crate::core::keywords::{is_valid_rust_ident_chars, python_ident, rust_raw_ident};
    let wire_name = serde_rename
        .map(|s| s.as_str())
        .or_else(|| config_renames.and_then(|r| r.get(field_name)).map(|s| s.as_str()))
        .unwrap_or(field_name);
    if is_valid_rust_ident_chars(wire_name) {
        // These path/self keywords cannot be raw identifiers in Rust. Keep
        // the serde wire key but use a legal constructor/stub parameter.
        if matches!(wire_name, "crate" | "self" | "Self" | "super") {
            return format!("{wire_name}_");
        }
        // The Rust escape wins when both apply. This is a RUST parameter identifier, and pyo3
        // strips the `r#` when deriving the Python-visible name -- so `r#type` satisfies both
        // sides, while `type_` satisfies neither the Rust parser nor a caller passing the wire
        // name. The two escapes collide only for names in both keyword sets (`type`), and there
        // the Python entry is over-broad anyway: `type` is a Python SOFT keyword, illegal only in
        // a `type X = ...` statement and perfectly legal as a parameter. A name that is a Python
        // keyword alone (`global`) still takes the rename. ~keep
        let rust_escaped = rust_raw_ident(wire_name);
        if rust_escaped == wire_name {
            rust_raw_ident(&python_ident(wire_name))
        } else {
            rust_escaped
        }
    } else {
        python_ident(field_name)
    }
}

/// Whether a nested default is exposed as a None-able constructor argument.
/// Shared with stubs so omission is not confused with accepting Python None.
pub(in crate::backends::pyo3) fn should_option_for_nested_default(
    typ: &TypeDef,
    field: &FieldDef,
    api: &ApiSurface,
) -> bool {
    if !typ.has_default || field.optional || matches!(&field.ty, TypeRef::Optional(_)) {
        return false;
    }
    let TypeRef::Named(ref type_name) = field.ty else {
        return false;
    };
    if api.types.iter().any(|t| t.name == *type_name && t.has_default) {
        return true;
    }
    field.default.as_deref() == Some("/* serde(default) */")
        && api
            .enums
            .iter()
            .any(|e| e.name == *type_name && crate::codegen::generators::enum_has_data_variants(e))
}

/// Replace the constructor in an impl block with one that honors serde_rename.
/// For has_default types, the constructor parameters should use serde_rename names
/// (the JSON wire names) to match other language bindings' public APIs.
/// This function finds the existing constructor and replaces it with a custom one.
/// Also adds extra parameters for any options-field bridges (e.g., visitor).
#[allow(clippy::too_many_arguments)]
pub(super) fn replace_constructor_with_serde_rename(
    impl_block: &str,
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    config: &crate::codegen::generators::RustBindingConfig,
    config_renames: Option<&HashMap<String, String>>,
    trait_bridges: &[TraitBridgeConfig],
    never_skip_cfg_field_names: &[String],
    api: &ApiSurface,
) -> String {
    // emit a second field-based `#[new]` constructor — the static method will be emitted
    // as `#[staticmethod] pub fn new(...)` and PyO3 forbids two `new` registrations in
    let has_explicit_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if has_explicit_new {
        return impl_block.to_string();
    }

    let bridge_field_name = trait_bridges
        .iter()
        .find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.options_type.as_deref() == Some(&typ.name)
        })
        .and_then(|b| b.resolved_options_field());

    let mut sorted_fields: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| !f.binding_excluded && (f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name)))
        .filter(|f| bridge_field_name.is_none() || f.name != bridge_field_name.unwrap())
        .collect();
    sorted_fields.sort_by_key(|f| {
        (
            typ.is_constructor_extension(f),
            f.optional,
            typ.legacy_constructor_fields()
                .iter()
                .position(|name| name == &f.name)
                .unwrap_or(0),
        )
    });

    let mut bare_defaulted = vec![false; sorted_fields.len()];
    let mut suffix_defaulted = true;
    for idx in (0..sorted_fields.len()).rev() {
        let field = sorted_fields[idx];
        let eligible_bare_default = !typ.has_default && field.has_bare_serde_enum_default() && suffix_defaulted;
        bare_defaulted[idx] = eligible_bare_default;
        suffix_defaulted &= typ.has_default || field.optional || eligible_bare_default;
    }

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, TypeRef::Duration);

            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            let ty =
                if (f.optional || force_optional || nested_default_optional) && !matches!(f.ty, TypeRef::Optional(_)) {
                    format!("Option<{}>", mapper.map_type(&f.ty))
                } else {
                    mapper.map_type(&f.ty)
                };
            format!("{}: {}", param_ident, ty)
        })
        .collect();

    let bridge_param = trait_bridges
        .iter()
        .find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.options_type.as_deref() == Some(&typ.name)
        })
        .and_then(|b| {
            let param_name = b.param_name.as_deref()?;
            Some((param_name, b.type_alias.as_deref().unwrap_or("object")))
        });

    let mut defaults: Vec<String> = sorted_fields
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, TypeRef::Duration);

            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            if f.optional || force_optional || nested_default_optional {
                format!("{}=None", param_ident)
            } else if typ.has_default {
                format!("{}=Self::default().{}", param_ident, f.name)
            } else if bare_defaulted[idx] {
                format!("{}=Default::default()", param_ident)
            } else {
                param_ident
            }
        })
        .collect();

    insert_constructor_keyword_only_marker(typ, &sorted_fields, &mut defaults);

    let assignments: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded && (bridge_field_name.is_none() || f.name != bridge_field_name.unwrap()))
        .map(|f| {
            if f.cfg.is_some() && !never_skip_cfg_field_names.contains(&f.name) {
                if f.optional {
                    format!("{}: None", f.name)
                } else {
                    format!("{}: Default::default()", f.name)
                }
            } else {
                let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

                let nested_default_optional = should_option_for_nested_default(typ, f, api);

                let binding_field = crate::core::keywords::python_ident(&f.name);
                if nested_default_optional {
                    format!(
                        "{}: {}.unwrap_or_else(|| Self::default().{})",
                        binding_field, param_ident, binding_field
                    )
                } else if param_ident != binding_field {
                    format!("{}: {}", binding_field, param_ident)
                } else {
                    binding_field
                }
            }
        })
        .collect();

    let mut all_defaults = defaults.clone();
    let mut all_params = params.clone();
    if let Some((param_name, type_alias)) = bridge_param {
        let field_type = bridge_field_name
            .and_then(|fname| typ.fields.iter().find(|f| f.name == fname))
            .map(|f| mapper.map_type(&f.ty))
            .unwrap_or_else(|| type_alias.to_string());
        let param_type = if field_type.starts_with("Option<") {
            field_type
        } else {
            format!("Option<{}>", field_type)
        };
        all_params.push(format!("{}: {}", param_name, param_type));
        all_defaults.push(format!("{}=None", param_name));
    }

    let param_list = if all_params.join(", ").len() > 100 {
        format!("\n        {},\n    ", all_params.join(",\n        "))
    } else {
        all_params.join(", ")
    };

    let mut all_assignments = assignments.clone();
    if let Some((param_name, _)) = bridge_param
        && let Some(field_name) = bridge_field_name
    {
        all_assignments.push(format!("{}: {}", field_name, param_name));
    }

    let json_checks = json_field_checks(&sorted_fields, config_renames, never_skip_cfg_field_names);
    let new_constructor = if json_checks.is_empty() {
        format!(
            "    #[allow(clippy::too_many_arguments)]\n    \
             #[must_use]\n    \
             #[pyo3(signature = ({}))]#[new]\n    \
             pub fn new({}) -> Self {{\n        \
             Self {{ {} }}\n    \
             }}",
            all_defaults.join(", "),
            param_list,
            all_assignments.join(", ")
        )
    } else {
        format!(
            "    #[allow(clippy::too_many_arguments)]\n    \
             #[pyo3(signature = ({}))]#[new]\n    \
             pub fn new({}) -> pyo3::PyResult<Self> {{\n{}        \
             Ok(Self {{ {} }})\n    \
             }}",
            all_defaults.join(", "),
            param_list,
            json_checks,
            all_assignments.join(", ")
        )
    };

    if let Some(start) = impl_block.find("#[pyo3(signature = (")
        && let Some(new_start) = impl_block[..start].rfind("\n")
        && let Some(fn_new_pos) = impl_block.find("pub fn new(")
    {
        let mut brace_count = 0;
        let mut in_fn = false;
        let mut end_pos = None;

        for (i, c) in impl_block[fn_new_pos..].chars().enumerate() {
            if c == '{' {
                in_fn = true;
                brace_count += 1;
            } else if c == '}' && in_fn {
                brace_count -= 1;
                if brace_count == 0 {
                    end_pos = Some(fn_new_pos + i + 1);
                    break;
                }
            }
        }

        if let Some(end) = end_pos {
            let before = &impl_block[..new_start + 1];
            let after = &impl_block[end..];
            return format!("{}{}{}", before, new_constructor, after);
        }
    }

    impl_block.to_string()
}

/// Statements that reject a JSON-typed constructor argument that is not valid JSON.
///
/// A JSON field crosses the boundary as a `str`. The conversion to the core type parses it with
/// `serde_json::from_str(..).ok()`, which turns a bad string into `None` and sends the request
/// without the field. The constructor is the only way to set the field of a frozen class, so it
/// parses the string once and raises `ValueError` with the field name. ~keep
fn json_field_checks(
    fields: &[&FieldDef],
    config_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
) -> String {
    let mut out = String::new();
    for field in fields {
        let is_json = match &field.ty {
            TypeRef::Json => true,
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Json),
            _ => false,
        };
        if !is_json || (field.cfg.is_some() && !never_skip_cfg_field_names.contains(&field.name)) {
            continue;
        }
        let ident = resolve_param_ident(&field.name, field.serde_rename.as_ref(), config_renames);
        let public = crate::core::keywords::python_ident(&field.name);
        let optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
        let check = format!(
            "serde_json::from_str::<serde_json::Value>({{value}}).map_err(|error| \
             pyo3::exceptions::PyValueError::new_err(format!(\"invalid JSON in `{public}`: {{error}}\")))?;"
        );
        if optional {
            let call = check.replace("{value}", "json");
            out.push_str(&format!(
                "        if let Some(json) = &{ident} {{\n            {call}\n        }}\n"
            ));
        } else {
            let call = check.replace("{value}", &format!("&{ident}"));
            out.push_str(&format!("        {call}\n"));
        }
    }
    out
}

fn insert_constructor_keyword_only_marker(typ: &TypeDef, fields: &[&FieldDef], defaults: &mut Vec<String>) {
    if let Some(index) = fields.iter().position(|field| typ.is_constructor_extension(field)) {
        defaults.insert(index, "*".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::pyo3::gen_bindings::config::binding_config;
    use crate::backends::pyo3::type_map::Pyo3Mapper;

    fn json_constructor(fields: Vec<FieldDef>, has_default: bool) -> String {
        let typ = TypeDef {
            name: "ChatRequest".to_string(),
            has_default,
            fields,
            ..Default::default()
        };
        replace_constructor_with_serde_rename(
            "impl ChatRequest {\n    #[pyo3(signature = ())]#[new]\n    pub fn new() -> Self { Self {} }\n}",
            &typ,
            &Pyo3Mapper::new(),
            &binding_config("sample_core", true),
            None,
            &[],
            &[],
            &ApiSurface::default(),
        )
    }

    #[test]
    fn invalid_json_in_an_optional_json_field_raises_value_error() {
        let rendered = json_constructor(
            vec![FieldDef {
                name: "extra_body".to_string(),
                ty: TypeRef::Json,
                optional: true,
                ..Default::default()
            }],
            false,
        );

        assert!(rendered.contains("-> pyo3::PyResult<Self>"), "{rendered}");
        assert!(
            rendered.contains("if let Some(json) = &extra_body {"),
            "an omitted field is not parsed:\n{rendered}"
        );
        assert!(
            rendered.contains("serde_json::from_str::<serde_json::Value>(json)"),
            "{rendered}"
        );
        assert!(rendered.contains("PyValueError::new_err(format!(\"invalid JSON in `extra_body`: {error}\"))"));
        assert!(rendered.contains("Ok(Self { extra_body })"), "{rendered}");
    }

    #[test]
    fn invalid_json_in_a_required_json_field_raises_value_error() {
        let rendered = json_constructor(
            vec![FieldDef {
                name: "schema".to_string(),
                ty: TypeRef::Json,
                ..Default::default()
            }],
            false,
        );

        assert!(
            rendered.contains("serde_json::from_str::<serde_json::Value>(&schema)"),
            "{rendered}"
        );
        assert!(rendered.contains("invalid JSON in `schema`"), "{rendered}");
    }

    #[test]
    fn constructor_without_json_fields_stays_infallible() {
        let rendered = json_constructor(
            vec![FieldDef {
                name: "model".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            false,
        );

        assert!(rendered.contains("pub fn new(model: String) -> Self {"), "{rendered}");
        assert!(!rendered.contains("PyResult"), "{rendered}");
    }

    #[test]
    fn bare_serde_default_is_omittable_without_optionalizing_required_sibling() {
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            fields: vec![
                FieldDef {
                    name: "llm".to_string(),
                    ty: TypeRef::Named("LlmConfig".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "alt_text".to_string(),
                    ty: TypeRef::Named("CaptionAltTextMode".to_string()),
                    default: Some("/* serde(default) */".to_string()),
                    typed_default: Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string())),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rendered = replace_constructor_with_serde_rename(
            "impl CaptioningConfig {\n    #[pyo3(signature = (llm, alt_text))]#[new]\n    pub fn new(llm: LlmConfig, alt_text: CaptionAltTextMode) -> Self { Self { llm, alt_text } }\n}",
            &typ,
            &Pyo3Mapper::new(),
            &binding_config("sample_core", true),
            None,
            &[],
            &[],
            &ApiSurface::default(),
        );

        assert!(
            rendered.contains("#[pyo3(signature = (llm, alt_text=Default::default()))]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("pub fn new(llm: LlmConfig, alt_text: CaptionAltTextMode)"),
            "{rendered}"
        );
        assert!(!rendered.contains("llm=Default::default()"), "{rendered}");
    }

    #[test]
    fn bare_serde_default_before_required_field_preserves_order_and_stays_required() {
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            fields: vec![
                FieldDef {
                    name: "llm".to_string(),
                    ty: TypeRef::Named("LlmConfig".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "alt_text".to_string(),
                    ty: TypeRef::Named("CaptionAltTextMode".to_string()),
                    default: Some("/* serde(default) */".to_string()),
                    typed_default: Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string())),
                    ..Default::default()
                },
                FieldDef {
                    name: "provider".to_string(),
                    ty: TypeRef::Named("CaptionProvider".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rendered = replace_constructor_with_serde_rename(
            "impl CaptioningConfig {\n    #[pyo3(signature = (llm, alt_text, provider))]#[new]\n    pub fn new(llm: LlmConfig, alt_text: CaptionAltTextMode, provider: CaptionProvider) -> Self { Self { llm, alt_text, provider } }\n}",
            &typ,
            &Pyo3Mapper::new(),
            &binding_config("sample_core", true),
            None,
            &[],
            &[],
            &ApiSurface::default(),
        );

        assert!(
            rendered.contains("#[pyo3(signature = (llm, alt_text, provider))]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("pub fn new(llm: LlmConfig, alt_text: CaptionAltTextMode, provider: CaptionProvider)"),
            "{rendered}"
        );
    }
}
