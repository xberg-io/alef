//! The C#-visible registration API the trait-bridge emitter produces, reported to the
//! reference-doc renderer.
//!
//! `trait_registry_class.jinja` is the emitter; the helpers here name the same class and methods
//! so `CsharpBackend::trait_bridge_registration_surface` cannot describe an API the emitted
//! `static class` does not declare.

use crate::codegen::naming::csharp_type_name;
use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::ApiSurface;

/// The static class the registration methods live on, e.g. `SamplePluginRegistry`.
pub(crate) fn registry_class_name(trait_name: &str) -> String {
    format!("{}Registry", csharp_type_name(trait_name))
}

/// `Register` is suffixed with the trait so several registries can be `using static`-imported
/// side by side; `Unregister` and `Clear` are not, because they take no implementation argument
/// that would collide. ~keep
pub(crate) fn register_method_name(trait_name: &str) -> String {
    format!("Register{}", csharp_type_name(trait_name))
}

/// See [`register_method_name`]. ~keep
pub(crate) const UNREGISTER_METHOD: &str = "Unregister";
/// See [`register_method_name`]. ~keep
pub(crate) const CLEAR_METHOD: &str = "Clear";

/// A bridge that declares both associated types takes the visitor path in
/// `gen_trait_bridges_file`, which emits no registry class at all.
fn is_visitor_bridge(bridge: &TraitBridgeConfig) -> bool {
    bridge.context_type.is_some() && bridge.result_type.is_some()
}

/// The registry classes and methods `gen_trait_bridges_file` emits for `config`.
///
/// Mirrors that function's gates: the trait must resolve in the `ApiSurface`, the bridge must
/// not exclude `csharp`, and a visitor bridge is skipped. `Register` is unconditional —
/// `register_fn` is not consulted, since the C# P/Invoke symbol is synthesised from the trait
/// name (see the KNOWN DIVERGENCE note in `trait_bridge.rs`). ~keep
pub fn registration_surface(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<TraitBridgeRegistrationSurface> {
    config
        .trait_bridges_for(Language::Csharp)
        .filter(|bridge| bridge.is_active_for("csharp"))
        .filter(|bridge| !is_visitor_bridge(bridge))
        .filter_map(|bridge| {
            let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge.trait_name)?;
            let registry = registry_class_name(&trait_def.name);
            Some(TraitBridgeRegistrationSurface {
                trait_name: trait_def.name.clone(),
                register_symbol: Some(format!("{registry}.{}", register_method_name(&trait_def.name))),
                unregister_symbol: bridge
                    .unregister_fn
                    .as_ref()
                    .map(|_| format!("{registry}.{UNREGISTER_METHOD}")),
                clear_symbol: bridge.clear_fn.as_ref().map(|_| format!("{registry}.{CLEAR_METHOD}")),
            })
        })
        .collect()
}
