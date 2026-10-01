use super::{TraitBridgeGenerator, TraitBridgeSpec};

pub fn gen_bridge_registration_fn(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    if let Some(code) = super::generator::disabled_code(generator) {
        return Some(code);
    }
    spec.bridge_config.register_fn.as_deref()?;
    Some(generator.gen_registration_fn(spec))
}

/// Generate the `unregister_xxx(name)` function that removes a previously
/// registered plugin from the registry.
///
/// Returns `None` when `bridge_config.unregister_fn` is absent or when the
/// backend hasn't opted in (returns the empty string from
/// [`TraitBridgeGenerator::gen_unregistration_fn`]).
pub fn gen_bridge_unregistration_fn(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    if let Some(code) = super::generator::disabled_code(generator) {
        return Some(code);
    }
    spec.bridge_config.unregister_fn.as_deref()?;
    let body = generator.gen_unregistration_fn(spec);
    if body.is_empty() { None } else { Some(body) }
}

/// Generate the `clear_xxx()` function that removes all registered plugins
/// of this type.
///
/// Returns `None` when `bridge_config.clear_fn` is absent or when the
/// backend hasn't opted in (returns the empty string from
/// [`TraitBridgeGenerator::gen_clear_fn`]).
pub fn gen_bridge_clear_fn(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> Option<String> {
    if let Some(code) = super::generator::disabled_code(generator) {
        return Some(code);
    }
    spec.bridge_config.clear_fn.as_deref()?;
    let body = generator.gen_clear_fn(spec);
    if body.is_empty() { None } else { Some(body) }
}

/// Resolve the FQN of a host-crate registry function (e.g.
/// `sample_core::registry::widgets::unregister_widget_backend`) given the bridge's
/// `registry_getter` path. The convention used by host crates is:
///
/// - `registry_getter = "sample_core::registry::get_widget_backend_registry"`
/// - top-level fn      = `sample_core::registry::widgets::unregister_widget_backend`
///
/// We rewrite `::registry::get_*_registry` to `::<sub>::<fn_name>` where
/// `<sub>` is the trait submodule name (extracted from `_*_registry`).
/// When the heuristic fails (no `registry_getter`, unexpected shape), we
/// fall back to `{core_import}::plugins::{fn_name}` so the user can rely on
/// a re-export.
///
/// Shared by every backend that opts in to `unregister_*`/`clear_*` codegen
/// (pyo3, napi, magnus, php, rustler, gleam, extendr, dart, swift, kotlin,
/// wasm). Replaces the duplicated `<lang>_host_function_path` helpers that
/// each backend used to define.
pub fn host_function_path(spec: &TraitBridgeSpec, fn_name: &str) -> String {
    if let Some(getter) = spec.bridge_config.registry_getter.as_deref() {
        let last = getter.rsplit("::").next().unwrap_or("");
        if let Some(sub) = last.strip_prefix("get_").and_then(|s| s.strip_suffix("_registry")) {
            let prefix_end = getter.len() - last.len();
            let prefix = &getter[..prefix_end];
            let prefix = prefix.trim_end_matches("registry::");
            return format!("{prefix}{sub}::{fn_name}");
        }
    }
    format!("{}::plugins::{}", spec.core_import, fn_name)
}
