//! Names of the Go trait-bridge registration API, and the surface report built from them.
//!
//! `orchestration::gen_trait_bridge` and `registration` emit these functions;
//! `GoBackend::trait_bridge_registration_surface` reports them, so both spell the names here.

use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;

/// `Register<Trait>` — the always-emitted registration entry point.
///
/// Takes the IR trait name unchanged rather than re-casing it: the emitted Go interface the
/// function accepts is declared under that same spelling, so a case transform here would
/// produce a function whose parameter type does not exist. ~keep
pub(crate) fn register_fn_name(trait_name: &str) -> String {
    format!("Register{trait_name}")
}

/// `Unregister<Trait>` — always emitted alongside [`register_fn_name`], independent of whether
/// `unregister_fn` is configured. See that function for why the trait name is not re-cased.
pub(crate) fn unregister_fn_name(trait_name: &str) -> String {
    format!("Unregister{trait_name}")
}

/// The extra, config-named unregistration wrapper `registration::gen_unregistration_fn` emits
/// when `unregister_fn` names something other than the standard `Unregister<Trait>`.
pub(crate) fn configured_unregister_fn_name(unregister_fn: &str) -> String {
    unregister_fn.to_pascal_case()
}

/// The clear-all wrapper's Go name, derived from the configured `clear_fn`.
pub(crate) fn clear_fn_name(clear_fn: &str) -> String {
    clear_fn.to_pascal_case()
}

/// The registration API the Go backend emits into `trait_bridges.go`.
///
/// Empty unless at least one bridge configures `register_fn`: `GoBackend::generate_bindings`
/// writes `trait_bridges.go` only then, so with none configured Go has no registration API at
/// all. Per bridge it mirrors `orchestration::gen_trait_bridges_file`: the bridge must not
/// exclude `go`, and its trait must be present in the `ApiSurface` — that lookup deliberately
/// does not require `is_trait`, matching the emitter. ~keep
pub fn registration_surface(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<TraitBridgeRegistrationSurface> {
    if !config
        .trait_bridges_for(Language::Go)
        .any(|bridge| bridge.register_fn.is_some())
    {
        return Vec::new();
    }
    config
        .trait_bridges_for(Language::Go)
        .filter_map(|bridge| {
            let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name)?;
            Some(TraitBridgeRegistrationSurface {
                trait_name: trait_def.name.clone(),
                register_symbol: Some(register_fn_name(&trait_def.name)),
                unregister_symbol: Some(unregister_fn_name(&trait_def.name)),
                clear_symbol: bridge.clear_fn.as_deref().map(clear_fn_name),
            })
        })
        .collect()
}
