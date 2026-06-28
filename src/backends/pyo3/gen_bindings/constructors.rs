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
    // When the type already has an explicit static `new()` method in its IR, do not
    // emit a second field-based `#[new]` constructor — the static method will be emitted
    // as `#[staticmethod] pub fn new(...)` and PyO3 forbids two `new` registrations in
    // the same impl block (E0592 duplicate definitions).
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
        // A nested struct with its own `Default`.
        if api.types.iter().any(|t| t.name == *type_name && t.has_default) {
            return true;
        }
        // A data enum (tagged union — it lives in `api.enums`, not `api.types`) carried with a serde
        // default. Such a field is None-able in the public surface, so its `#[new]` param is `Option<T>`
        // (None falls back to the core default via `unwrap_or_else`). This lets the converter pass the
        // coerced value or None directly rather than a conditional `**{...}` keyword-spread that no
        // type checker can verify.
        field.default.as_deref() == Some("/* serde(default) */")
            && api
                .enums
                .iter()
                .any(|e| e.name == *type_name && crate::codegen::generators::enum_has_data_variants(e))
    }

    // Check if this type has an options-field bridge (e.g., ParseOptions.visitor).
    // The bridge field is appended later via `bridge_param`; filter it out of
    // `sorted_fields` so we do not emit it twice when the field is force-restored
    // through `never_skip_cfg_field_names`.
    let bridge_field_name = trait_bridges
        .iter()
        .find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.options_type.as_deref() == Some(&typ.name)
        })
        .and_then(|b| b.resolved_options_field());

    // Build parameter list with serde_rename and config-based renames.
    // Include cfg-gated fields that the consumer has force-restored via
    // `never_skip_cfg_field_names` (e.g. sample_core-py builds with all features so
    // pdf_options / keywords / html_* / layout / tree_sitter need to be kwargs).
    let mut sorted_fields: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| !f.binding_excluded && (f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name)))
        .filter(|f| bridge_field_name.is_none() || f.name != bridge_field_name.unwrap())
        .collect();
    sorted_fields.sort_by_key(|f| f.optional as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            // Use serde_rename if available (and valid), otherwise the Rust field name.
            // Keywords are escaped as raw identifiers (e.g. "type" → "r#type").
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            // Determine if this field should be optional in the constructor.
            // This matches the logic in gen_struct_with_per_field_attrs (structs.rs lines 128-131).
            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, TypeRef::Duration);

            // BLK-5 fix: for non-optional nested-struct fields on has_default types,
            // if the nested struct also has_default, emit as Option<T> to accept None.
            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            let ty =
                if (f.optional || force_optional || nested_default_optional) && !matches!(f.ty, TypeRef::Optional(_)) {
                    // All optional constructor parameters are emitted as Option<T>.
                    // The IR unwraps TypeRef::Optional to mark fields as optional,
                    // so we need to re-wrap the base type for the constructor signature.
                    // Skip re-wrapping when the IR field type is *already* Optional —
                    // that happens for Update structs where a source field of
                    // `Option<Option<T>>` peels to `f.optional = true, f.ty = Optional(T)`.
                    // Mirrors the same guard in `gen_struct_with_per_field_attrs`.
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
            // PyO3 strips the `r#` prefix when deriving the Python-facing keyword argument
            // name, so `r#type` in the signature → Python `type`.
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            // Same force_optional logic as above.
            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, TypeRef::Duration);

            // BLK-5 fix: for non-optional nested-struct fields on has_default types,
            // if the nested struct also has_default, emit default as None.
            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            if f.optional || force_optional || nested_default_optional {
                format!("{}=None", param_ident)
            } else if typ.has_default {
                // For has_default types, non-optional fields get a default value in the signature
                // so the generated `__new__` is callable with keyword args omitted.
                // The field's default is `Self::default().<field>`.
                format!("{}=Self::default().{}", param_ident, f.name)
            } else {
                // For non-has_default types, required fields have no default in the signature
                // (they are required keyword arguments).
                param_ident
            }
        })
        .collect();

    // Struct literal uses bare Rust field names (never renamed).
    // For non-cfg fields: use constructor parameters (with explicit form if renamed).
    // For cfg-gated fields: initialize with default (None for Option types, Default::default() otherwise).
    // Bridge fields are handled separately below.
    let assignments: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded && (bridge_field_name.is_none() || f.name != bridge_field_name.unwrap()))
        .map(|f| {
            if f.cfg.is_some() && !never_skip_cfg_field_names.contains(&f.name) {
                // Cfg-gated field that was NOT force-restored: not a constructor parameter, use default
                if f.optional {
                    format!("{}: None", f.name)
                } else {
                    format!("{}: Default::default()", f.name)
                }
            } else {
                // Non-cfg field: use constructor parameter.
                // Use the same resolve_param_ident logic so the struct literal references
                // exactly the same variable as the parameter declaration.
                let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

                // BLK-5 fix: for nested-struct fields emitted as Option<T> due to has_default,
                // use unwrap_or_else to fall back to the nested type's default.
                let nested_default_optional = should_option_for_nested_default(typ, f, api);

                // The binding struct's Rust field name is python-keyword-escaped
                // (e.g. `from` -> `from_`), so the LEFT side of the struct literal must
                // match that escaped name, not the core IR field name.
                let binding_field = crate::core::keywords::python_ident(&f.name);
                if nested_default_optional {
                    // Use unwrap_or_else for nested default optional fields
                    format!(
                        "{}: {}.unwrap_or_else(|| Self::default().{})",
                        binding_field, param_ident, binding_field
                    )
                } else if param_ident != binding_field {
                    // Parameter name differs from binding struct field name:
                    // use explicit form to match the parameter variable
                    format!("{}: {}", binding_field, param_ident)
                } else {
                    // Names match: use shorthand
                    binding_field
                }
            }
        })
        .collect();

    // Add bridge parameter to defaults and params if present.
    //
    // The bridge parameter's *type* must match the struct field it ultimately
    // populates — the struct literal below emits a bare `visitor: visitor`,
    // which would fail to compile if the parameter type and field type differ
    // (e.g. user-facing `VisitorHandle` pyclass vs the binding's internal
    // `PyVisitorRef` wrapper). Look up the matching field's actual mapped
    // type and use it. Fall back to the bridge's `type_alias` only when the
    // bridge field can't be located in the struct (which would be a bug, but
    // preserves the prior behaviour).
    let mut all_defaults = defaults.clone();
    let mut all_params = params.clone();
    if let Some((param_name, type_alias)) = bridge_param {
        let field_type = bridge_field_name
            .and_then(|fname| typ.fields.iter().find(|f| f.name == fname))
            .map(|f| mapper.map_type(&f.ty))
            .unwrap_or_else(|| type_alias.to_string());
        // The field's mapped type may already include `Option<...>` (it
        // typically does, since bridge fields are optional). Avoid double-
        // wrapping by checking for the prefix.
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

    // Build the assignment for the bridge field
    let mut all_assignments = assignments.clone();
    if let Some((param_name, _)) = bridge_param {
        if let Some(field_name) = bridge_field_name {
            all_assignments.push(format!("{}: {}", field_name, param_name));
        }
    }

    // Build the new constructor method (without impl wrapper — we'll inject it into existing impl)
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

    // Find and replace the old constructor in the impl block
    // Look for the pattern that includes the signature and fn new
    if let Some(start) = impl_block.find("#[pyo3(signature = (") {
        if let Some(new_start) = impl_block[..start].rfind("\n") {
            // Find the end of the constructor (closing brace of the function)
            if let Some(fn_new_pos) = impl_block.find("pub fn new(") {
                // Find the closing brace of this constructor
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

    // Fallback: if we can't find the constructor to replace, return the original
    impl_block.to_string()
}
