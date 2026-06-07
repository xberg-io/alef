use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, TypeDef, TypeRef};

pub fn bridge_handle_path(api: &ApiSurface, bridge: &TraitBridgeConfig, core_import: &str) -> String {
    let alias = bridge.type_alias.as_deref().unwrap_or(&bridge.trait_name);
    api.types
        .iter()
        .find(|typ| typ.name == alias && !typ.rust_path.is_empty())
        .map(|typ| typ.rust_path.replace('-', "_"))
        .or_else(|| api.excluded_type_paths.get(alias).map(|path| path.replace('-', "_")))
        .unwrap_or_else(|| format!("{core_import}::{alias}"))
}

/// Generate a backend visitor bridge wrapper name from configuration.
pub fn bridge_wrapper_name(prefix: &str, bridge: &TraitBridgeConfig) -> String {
    format!("{}{}Bridge", prefix, bridge.trait_name)
}

/// Return true when a type reference points at any configured bridge handle alias.
pub fn is_bridge_handle_type_ref(ty: &TypeRef, bridges: &[TraitBridgeConfig]) -> bool {
    bridges
        .iter()
        .filter_map(|bridge| bridge.type_alias.as_deref())
        .any(|alias| field_type_matches_alias(ty, alias))
}

/// Return true when a function name is emitted by trait-bridge codegen.
pub fn is_trait_bridge_managed_fn(func_name: &str, bridges: &[TraitBridgeConfig]) -> bool {
    bridges.iter().any(|b| b.clear_fn.as_deref() == Some(func_name))
}

/// Find the first function parameter that matches a trait bridge configuration
/// (by type alias or parameter name).
///
/// Bridges configured with `bind_via = "options_field"` are skipped — they live on a
/// struct field rather than directly as a parameter, and are returned by
/// [`find_bridge_field`] instead.
pub fn find_bridge_param<'a>(
    func: &FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for (idx, param) in func.params.iter().enumerate() {
        let named = match &param.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n.as_str())
                } else {
                    None
                }
            }
            _ => None,
        };
        for bridge in bridges {
            if bridge.bind_via != BridgeBinding::FunctionParam {
                continue;
            }
            if let Some(type_name) = named {
                if bridge.type_alias.as_deref() == Some(type_name) {
                    return Some((idx, bridge));
                }
            }
            if bridge.param_name.as_deref() == Some(param.name.as_str()) {
                return Some((idx, bridge));
            }
        }
    }
    None
}

/// Match info for a trait bridge whose handle lives as a struct field
/// (`bind_via = "options_field"`).
#[derive(Debug, Clone)]
pub struct BridgeFieldMatch<'a> {
    /// Index of the function parameter that carries the owning struct.
    pub param_index: usize,
    /// Name of the parameter (e.g., `"options"`).
    pub param_name: String,
    /// IR type name of the parameter, with any `Option<>` wrapper unwrapped.
    pub options_type: String,
    /// True if the param is `Option<TypeName>` rather than `TypeName`.
    pub param_is_optional: bool,
    /// Name of the field on `options_type` that holds the bridge handle.
    pub field_name: String,
    /// The matching field definition (carries the field's `TypeRef`).
    pub field: &'a FieldDef,
    /// The bridge configuration that produced the match.
    pub bridge: &'a TraitBridgeConfig,
}

/// Find the first function parameter whose IR type carries a bridge field
/// (`bind_via = "options_field"`).
///
/// For each function parameter whose IR type is `Named(N)` or `Optional<Named(N)>`,
/// look up `N` in `types`. If `N` matches any bridge's `options_type`, search its
/// fields for one whose name matches the bridge's resolved options field (or whose
/// type's `Named` alias matches the bridge's `type_alias`). Returns the first match.
///
/// Bridges configured with `bind_via = "function_param"` are skipped — those go
/// through [`find_bridge_param`] instead.
pub fn find_bridge_field<'a>(
    func: &FunctionDef,
    types: &'a [TypeDef],
    bridges: &'a [TraitBridgeConfig],
) -> Option<BridgeFieldMatch<'a>> {
    fn unwrap_named(ty: &TypeRef) -> Option<(&str, bool)> {
        match ty {
            TypeRef::Named(n) => Some((n.as_str(), false)),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some((n.as_str(), true))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    for (idx, param) in func.params.iter().enumerate() {
        let Some((type_name, is_optional)) = unwrap_named(&param.ty) else {
            continue;
        };
        let Some(type_def) = types.iter().find(|t| t.name == type_name) else {
            continue;
        };
        for bridge in bridges {
            if bridge.bind_via != BridgeBinding::OptionsField {
                continue;
            }
            if bridge.options_type.as_deref() != Some(type_name) {
                continue;
            }
            let field_name = bridge.resolved_options_field();
            for field in &type_def.fields {
                let matches_name = field_name.is_some_and(|n| field.name == n);
                let matches_alias = bridge
                    .type_alias
                    .as_deref()
                    .is_some_and(|alias| field_type_matches_alias(&field.ty, alias));
                if matches_name || matches_alias {
                    return Some(BridgeFieldMatch {
                        param_index: idx,
                        param_name: param.name.clone(),
                        options_type: type_name.to_string(),
                        param_is_optional: is_optional,
                        field_name: field.name.clone(),
                        field,
                        bridge,
                    });
                }
            }
        }
    }
    None
}

/// True if `field_ty` references a `Named` type whose name equals `alias`,
/// allowing for `Option<>` and `Vec<>` wrappers.
fn field_type_matches_alias(field_ty: &TypeRef, alias: &str) -> bool {
    match field_ty {
        TypeRef::Named(n) => n == alias,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_type_matches_alias(inner, alias),
        _ => false,
    }
}
