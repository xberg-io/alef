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
pub(in crate::backends::pyo3) fn resolve_param_ident<'a>(
    field_name: &'a str,
    serde_rename: Option<&'a String>,
    config_renames: Option<&HashMap<String, String>>,
) -> String {
    use crate::core::keywords::{is_valid_rust_ident_chars, rust_raw_ident};
    let wire_name = serde_rename
        .map(|s| s.as_str())
        .or_else(|| config_renames.and_then(|r| r.get(field_name)).map(|s| s.as_str()))
        .unwrap_or(field_name);
    if is_valid_rust_ident_chars(wire_name) {
        rust_raw_ident(wire_name)
    } else {
        field_name.to_string()
    }
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

    /// Check if a field should be emitted as Option<T> to accept None for BLK-5 fix.
    /// This applies when:
    /// - The parent type has_default=true
    /// - The field is non-optional (!f.optional && not already Optional)
    /// - The field type is a Named type
    /// - The referenced type has has_default=true
    fn should_option_for_nested_default(typ: &TypeDef, field: &FieldDef, api: &ApiSurface) -> bool {
        if !typ.has_default || field.optional || matches!(&field.ty, TypeRef::Optional(_)) {
            return false;
        }
        let TypeRef::Named(ref type_name) = field.ty else {
            return false;
        };
        if api.types.iter().any(|t| t.name == *type_name && t.has_default) {
            return true;
        }
        // default. Such a field is None-able in the public surface, so its `#[new]` param is `Option<T>`
        field.default.as_deref() == Some("/* serde(default) */")
            && api
                .enums
                .iter()
                .any(|e| e.name == *type_name && crate::codegen::generators::enum_has_data_variants(e))
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
    sorted_fields.sort_by_key(|f| f.optional as u8);

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

    let defaults: Vec<String> = sorted_fields
        .iter()
        .filter(|f| bridge_field_name.is_none() || f.name != bridge_field_name.unwrap())
        .map(|f| {
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
            } else {
                param_ident
            }
        })
        .collect();

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
    if let Some((param_name, _)) = bridge_param {
        if let Some(field_name) = bridge_field_name {
            all_assignments.push(format!("{}: {}", field_name, param_name));
        }
    }

    let new_constructor = format!(
        "    #[allow(clippy::too_many_arguments)]\n    \
         #[must_use]\n    \
         #[pyo3(signature = ({}))]#[new]\n    \
         pub fn new({}) -> Self {{\n        \
         Self {{ {} }}\n    \
         }}",
        all_defaults.join(", "),
        param_list,
        all_assignments.join(", ")
    );

    if let Some(start) = impl_block.find("#[pyo3(signature = (") {
        if let Some(new_start) = impl_block[..start].rfind("\n") {
            if let Some(fn_new_pos) = impl_block.find("pub fn new(") {
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
        }
    }

    impl_block.to_string()
}
