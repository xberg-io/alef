use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(super) struct LibSetupContext<'a> {
    pub(super) path_map: AHashMap<String, String>,
    pub(super) enum_names: AHashSet<String>,
    pub(super) ffi_param_enums: AHashSet<String>,
    pub(super) clone_names: AHashSet<String>,
    pub(super) serde_names: AHashSet<String>,
    pub(super) fields_c_types: Option<&'a std::collections::HashMap<String, String>>,
}

pub(super) fn build_lib_setup_context<'a>(api: &ApiSurface, config: &'a ResolvedCrateConfig) -> LibSetupContext<'a> {
    let mut path_map = AHashMap::new();
    for t in api.types.iter().filter(|t| !t.is_trait) {
        path_map.insert(t.name.clone(), t.rust_path.replace('-', "_"));
    }
    for e in &api.enums {
        path_map.insert(e.name.clone(), e.rust_path.replace('-', "_"));
    }
    for err in &api.errors {
        path_map.insert(err.name.clone(), err.rust_path.replace('-', "_"));
    }

    let enum_names = crate::backends::ffi::type_map::scalar_c_abi_named_types(api);

    let ffi_param_enums: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple))
        .map(|e| e.name.clone())
        .collect();

    let clone_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && t.is_clone && !t.is_copy)
        .map(|t| t.name.clone())
        .chain(api.enums.iter().filter(|e| !e.is_copy).map(|e| e.name.clone()))
        .chain(
            config
                .trait_bridges_for(Language::Ffi)
                .filter_map(|bridge| bridge.type_alias.clone()),
        )
        .collect();

    let serde_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_serde)
        .map(|t| t.name.clone())
        .chain(api.enums.iter().map(|e| e.name.clone()))
        .collect();

    LibSetupContext {
        path_map,
        enum_names,
        ffi_param_enums,
        clone_names,
        serde_names,
        fields_c_types: config.e2e.as_ref().map(|e2e| &e2e.fields_c_types),
    }
}

pub(super) fn named_type_ref(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_ref(inner),
        _ => None,
    }
}

pub(super) fn crosses_borrowed_handle_boundary(ty: &TypeRef, borrowed_types: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => borrowed_types.contains(name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => crosses_borrowed_handle_boundary(inner, borrowed_types),
        TypeRef::Map(key, value) => {
            crosses_borrowed_handle_boundary(key, borrowed_types)
                || crosses_borrowed_handle_boundary(value, borrowed_types)
        }
        _ => false,
    }
}

pub(super) fn has_trait_bridge_param(func: &FunctionDef, trait_bridges: &[TraitBridgeConfig]) -> bool {
    func.params.iter().any(|param| {
        let param_type = named_type_ref(&param.ty);
        trait_bridges.iter().any(|bridge| {
            bridge.bind_via != BridgeBinding::OptionsField
                && (bridge.param_name.as_deref() == Some(param.name.as_str())
                    || bridge.type_alias.as_deref() == param_type)
        })
    })
}

/// The parameter carrying an options-field bridge's owning struct, if any.
///
/// Delegates to the shared [`options_field_bridge_site`] predicate so the FFI wrapper cannot
/// disagree with the pyo3 wrapper, the `.pyi` stub or the `api.py` facade about which
/// functions gain a bridge — including the `exclude_functions` gate. Callers pass an
/// already `targets_ffi`-filtered bridge list. ~keep
pub(super) fn options_field_bridge_for_function<'a, I>(
    func: &'a FunctionDef,
    trait_bridges: I,
) -> Option<(&'a ParamDef, &'a str)>
where
    I: IntoIterator<Item = &'a TraitBridgeConfig>,
{
    let site = crate::codegen::generators::trait_bridge::options_field_bridge_site(func, trait_bridges)?;
    Some((site.param, site.options_type))
}

pub(super) fn function_param_bridge_for_visitor_callbacks<'a>(
    api: &'a ApiSurface,
    trait_bridges: &'a [TraitBridgeConfig],
) -> Option<(&'a TraitBridgeConfig, &'a FunctionDef)> {
    trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via != BridgeBinding::OptionsField)
        .find_map(|bridge| {
            api.functions
                .iter()
                .find(|func| has_trait_bridge_param(func, std::slice::from_ref(bridge)))
                .map(|func| (bridge, func))
        })
}
