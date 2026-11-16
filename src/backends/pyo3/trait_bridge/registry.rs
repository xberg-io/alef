use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// The bridges PyO3 actually emits: not excluded for this target, and the bridged trait resolves
/// in `api.types`. `gen_trait_bridge` (see `super::gen_trait_bridge`) runs only for this same set,
/// so every collector below must restrict itself to it or it names a symbol no pass wrote. ~keep
fn active_configs<'a>(
    configs: &'a [TraitBridgeConfig],
    api: &ApiSurface,
) -> impl Iterator<Item = &'a TraitBridgeConfig> {
    configs
        .iter()
        .filter(move |c| super::active_bridge_trait(c, api).is_some())
}

pub(crate) fn has_registry_wrappers(configs: &[TraitBridgeConfig], api: &ApiSurface) -> bool {
    active_configs(configs, api).any(|c| {
        crate::codegen::generators::trait_bridge::bridge_register_symbol(c).is_some()
            || c.unregister_fn.is_some()
            || c.clear_fn.is_some()
    })
}

/// Register function names for `#[pymodule]` wiring, restricted to bridges PyO3 actually emits.
///
/// `register_fn` alone is not enough: `Pyo3BridgeGenerator::gen_registration_fn` writes no
/// `#[pyfunction]` without `registry_getter` too — see
/// [`crate::codegen::generators::trait_bridge::bridge_register_symbol`]. Naming a function here
/// that no `#[pyfunction]` defines is a Rust `cannot find value` compile error, not a missing
/// binding. ~keep
pub fn collect_bridge_register_fns(configs: &[TraitBridgeConfig], api: &ApiSurface) -> Vec<String> {
    active_configs(configs, api)
        .filter_map(|c| crate::codegen::generators::trait_bridge::bridge_register_symbol(c))
        .map(str::to_owned)
        .collect()
}

/// Collect unregistration function names for api.py pass-through wrappers.
///
/// Only bridges that define an `unregister_fn` AND that PyO3 actually emits are included.
pub fn collect_bridge_unregister_fns(configs: &[TraitBridgeConfig], api: &ApiSurface) -> Vec<String> {
    active_configs(configs, api)
        .filter_map(|c| c.unregister_fn.clone())
        .collect()
}

/// Collect clear function names for api.py pass-through wrappers.
///
/// Only bridges that define a `clear_fn` AND that PyO3 actually emits are included.
pub fn collect_bridge_clear_fns(configs: &[TraitBridgeConfig], api: &ApiSurface) -> Vec<String> {
    active_configs(configs, api)
        .filter_map(|c| c.clear_fn.clone())
        .collect()
}

/// Imports needed by trait bridge generated code.
pub fn trait_bridge_imports(configs: &[TraitBridgeConfig]) -> Vec<&'static str> {
    if configs.is_empty() {
        return vec![];
    }
    vec![
        "use async_trait::async_trait;",
        "use pyo3::prelude::*;",
        "use std::sync::Arc;",
    ]
}
