use crate::codegen::builder::RustFileBuilder;
use crate::core::config::{CapsuleTypeConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::collections::HashMap;

#[cfg(test)]
mod tests;

pub(super) fn opaque_type_names(api: &ApiSurface, config: &ResolvedCrateConfig) -> AHashSet<String> {
    let mut names: AHashSet<String> = api
        .types
        .iter()
        .filter(|typ| typ.is_opaque)
        .map(|typ| typ.name.clone())
        .collect();
    for name in config.opaque_types.keys() {
        if !config
            .python
            .as_ref()
            .is_some_and(|python| python.capsule_types.contains_key(name))
        {
            names.insert(name.clone());
        }
    }
    names
}

pub(super) fn send_sync_type_names(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    absent_types: &AHashSet<String>,
) -> anyhow::Result<AHashSet<String>> {
    let names: AHashSet<String> = config
        .python
        .as_ref()
        .map(|python| python.send_sync_types.iter().cloned().collect())
        .unwrap_or_default();
    for name in &names {
        if absent_types.contains(name)
            || !api
                .types
                .iter()
                .any(|typ| typ.name == *name && typ.is_opaque && !typ.is_trait)
        {
            anyhow::bail!("python.send_sync_types names an unavailable extracted opaque type: {name}");
        }
    }
    Ok(names)
}

pub(crate) fn exclude_capsule_opaque_types(
    py_exclude_types: &mut AHashSet<String>,
    config: &ResolvedCrateConfig,
    capsule_types: &HashMap<String, CapsuleTypeConfig>,
) {
    for name in config.opaque_types.keys() {
        if capsule_types.contains_key(name) {
            py_exclude_types.insert(name.clone());
        }
    }
}

pub(crate) fn emit_wrappers(
    builder: &mut RustFileBuilder,
    config: &ResolvedCrateConfig,
    capsule_types: &HashMap<String, CapsuleTypeConfig>,
    emitted_pyclass_names: &AHashSet<&str>,
    error_type_names: &AHashSet<String>,
    api_opaque_types_empty: bool,
) {
    if config.opaque_types.is_empty() {
        return;
    }

    if api_opaque_types_empty {
        builder.add_import("std::sync::Arc");
    }

    let mut emitted_opaque_wrapper_names: AHashSet<&str> = AHashSet::new();
    for (name, source_path) in &config.opaque_types {
        if capsule_types.contains_key(name)
            || emitted_pyclass_names.contains(name.as_str())
            || error_type_names.contains(name.as_str())
            || !emitted_opaque_wrapper_names.insert(name.as_str())
        {
            continue;
        }

        let rust_path = source_path.replace('-', "_");
        let wrapper = crate::backends::pyo3::template_env::render(
            "config_opaque_wrapper.rs.jinja",
            crate::alef_context! {
                name => name,
                rust_path => rust_path,
            },
        );
        builder.add_item(wrapper.trim_end());
    }
}
