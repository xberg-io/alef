use crate::codegen::builder::RustFileBuilder;
use crate::core::config::{NodeCapsuleTypeConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::collections::HashMap;

pub(super) fn collect_opaque_types(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
) -> AHashSet<String> {
    let mut opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !capsule_types.contains_key(&t.name))
        .map(|t| t.name.clone())
        .collect();

    for name in config.opaque_types.keys() {
        if !capsule_types.contains_key(name) {
            opaque_types.insert(name.clone());
        }
    }

    opaque_types
}

pub(super) fn exclude_capsule_opaque_types(
    exclude_types: &mut AHashSet<String>,
    config: &ResolvedCrateConfig,
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
) {
    for name in config.opaque_types.keys() {
        if capsule_types.contains_key(name) {
            exclude_types.insert(name.clone());
        }
    }
}

pub(super) fn emit_wrappers(
    builder: &mut RustFileBuilder,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
    prefix: &str,
) {
    let emitted_type_names: AHashSet<&str> = api.types.iter().map(|typ| typ.name.as_str()).collect();

    for (name, source_path) in &config.opaque_types {
        if capsule_types.contains_key(name) || emitted_type_names.contains(name.as_str()) {
            continue;
        }

        let rust_path = source_path.replace('-', "_");
        let struct_name = format!("{prefix}{name}");
        let wrapper = crate::backends::napi::template_env::render(
            "config_opaque_wrapper.rs.jinja",
            crate::alef_context! {
                name => name,
                rust_path => rust_path,
                struct_name => struct_name,
            },
        );
        builder.add_item(wrapper.trim_end());
    }
}
