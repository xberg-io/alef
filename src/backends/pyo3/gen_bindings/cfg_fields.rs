use crate::codegen::shared::binding_fields;
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

pub(super) fn never_skip_cfg_field_names(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let mut field_names: Vec<String> = config
        .trait_bridges
        .iter()
        .filter_map(|bridge| {
            if bridge.bind_via == BridgeBinding::OptionsField {
                bridge.resolved_options_field().map(str::to_string)
            } else {
                None
            }
        })
        .collect();

    for typ in api.types.iter().filter(|typ| typ.has_default && !typ.is_trait) {
        for field in binding_fields(&typ.fields) {
            let Some(cfg) = field.cfg.as_deref() else {
                continue;
            };
            let present = !typ.has_stripped_cfg_fields || super::config::cfg_present_for_pyo3(cfg);
            if present && !field_names.contains(&field.name) {
                field_names.push(field.name.clone());
            }
        }
    }

    field_names
}

pub(super) fn default_required_types(api: &ApiSurface) -> AHashSet<&str> {
    api.types
        .iter()
        .filter(|typ| typ.has_default)
        .flat_map(|typ| typ.fields.iter())
        .filter(|field| !field.optional)
        .filter_map(|field| match &field.ty {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

pub(super) fn py_field_renames(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashMap<String, String> {
    let mut renames = HashMap::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for field in binding_fields(&typ.fields) {
            if let Some(escaped) = config.resolve_field_name(Language::Python, &typ.name, &field.name) {
                renames.insert(format!("{}.{}", typ.name, field.name), escaped);
            }
        }
    }
    renames
}
