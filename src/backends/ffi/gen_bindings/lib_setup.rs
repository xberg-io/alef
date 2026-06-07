use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
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

    let enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.is_copy)
        .map(|e| e.name.clone())
        .chain(
            api.types
                .iter()
                .filter(|t| !t.is_trait && t.is_copy)
                .map(|t| t.name.clone()),
        )
        .collect();

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

pub(super) fn options_field_bridge_for_function<'a>(
    func: &'a FunctionDef,
    trait_bridges: &'a [TraitBridgeConfig],
) -> Option<(&'a ParamDef, &'a str)> {
    trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via == BridgeBinding::OptionsField)
        .find_map(|bridge| {
            let options_type = bridge.options_type.as_deref()?;
            let options_param = func
                .params
                .iter()
                .find(|param| named_type_ref(&param.ty) == Some(options_type))?;
            Some((options_param, options_type))
        })
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
