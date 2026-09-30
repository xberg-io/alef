//! Rust-side trait-bridge helpers for the WASM backend that are not themselves emitters:
//! a post-processing fixup over generated builder calls, and the registration surface reported
//! to the reference-doc renderer.

use crate::codegen::generators::trait_bridge::to_camel_case;
use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};

/// The `exclude_languages` spellings that name this target. WASM has no second spelling — the
/// language and the backend are both `"wasm"` — but the gate is expressed as a list so it reads
/// the same here as in the backends that do. ~keep
const TARGET_SPELLINGS: [&str; 1] = ["wasm"];

/// Whether `bridge` is emitted for the WASM target at all.
pub(super) fn targets_wasm(bridge: &TraitBridgeConfig) -> bool {
    crate::codegen::generators::trait_bridge::bridge_targets_language(bridge, &TARGET_SPELLINGS)
}

/// The configured bridges WASM actually emits, in configuration order.
///
/// Every site that enumerates bridges — the `#[wasm_bindgen]` items, the options-field wiring,
/// the opaque-alias set, and `WasmBackend::trait_bridge_registration_surface` — iterates this,
/// so no pass can wire up a bridge another pass skipped. ~keep
pub(super) fn active_bridges(config: &ResolvedCrateConfig) -> impl Iterator<Item = &TraitBridgeConfig> {
    config.trait_bridges_for(Language::Wasm)
}

/// The trait a bridge wraps, when WASM emits that bridge at all.
pub(super) fn active_bridge_trait<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}

pub(super) fn forward_trait_bridge_builder_fields(mut content: String, config: &ResolvedCrateConfig) -> String {
    for bridge in active_bridges(config) {
        if let Some(field_name) = bridge.resolved_options_field() {
            let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
            let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
            let replacement = format!(".{}({}.map(|v| (*v.inner).clone()))", field_name, param_name);
            content = content.replace(&pattern, &replacement);
        }
    }
    content
}

/// The JS names `trait_bridge::gen_trait_bridge` exports for each configured bridge.
///
/// `to_camel_case` is the same helper the emitter feeds into `#[wasm_bindgen(js_name = ...)]`.
/// Registration additionally requires `registry_getter`, without which
/// `WasmBridgeGenerator::gen_registration_fn` emits nothing.
///
/// `active_bridge_trait` is the same gate the emission path applies before calling
/// `gen_trait_bridge`, so an `exclude_languages`-suppressed bridge is absent from both. ~keep
pub(super) fn registration_surface(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> Vec<TraitBridgeRegistrationSurface> {
    config
        .trait_bridges_for(Language::Wasm)
        .filter_map(|bridge| {
            let trait_def = active_bridge_trait(bridge, api)?;
            // A visitor bridge takes `gen_visitor_bridge`, which emits no registry API. ~keep
            let is_visitor_bridge = bridge.type_alias.is_some()
                && bridge.register_fn.is_none()
                && bridge.super_trait.is_none()
                && trait_def.methods.iter().all(|method| method.has_default_impl);
            if is_visitor_bridge {
                return None;
            }
            let surface = TraitBridgeRegistrationSurface {
                trait_name: trait_def.name.clone(),
                register_symbol: bridge
                    .register_fn
                    .as_deref()
                    .filter(|_| bridge.registry_getter.is_some())
                    .map(to_camel_case),
                unregister_symbol: bridge.unregister_fn.as_deref().map(to_camel_case),
                clear_symbol: bridge.clear_fn.as_deref().map(to_camel_case),
            };
            let emits_nothing = surface.register_symbol.is_none()
                && surface.unregister_symbol.is_none()
                && surface.clear_symbol.is_none();
            (!emits_nothing).then_some(surface)
        })
        .collect()
}
