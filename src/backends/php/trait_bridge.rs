mod bridge_function;
mod generator;
mod interfaces;

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
pub use bridge_function::gen_bridge_function;
pub use generator::{PhpBridgeGenerator, gen_trait_bridge};
pub use interfaces::{gen_registration_interface, gen_visitor_interface, visitor_interface_class_name};

use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};

pub(crate) const DISABLED_MESSAGE: &str = "PHP callback bridges are disabled because Zend values are request-thread-bound and cannot safely implement Send or Sync; exclude PHP from this bridge";

pub(crate) fn disabled_code() -> String {
    format!("compile_error!({DISABLED_MESSAGE:?});")
}

/// The `exclude_languages` spellings that name this target. PHP has no second spelling — the
/// language and the backend are both `"php"` — but the gate is expressed as a list so it reads
/// the same here as in the backends that do. ~keep
const TARGET_SPELLINGS: [&str; 1] = ["php"];

/// Whether `bridge` is emitted for the PHP target at all.
pub fn targets_php(bridge: &TraitBridgeConfig) -> bool {
    crate::codegen::generators::trait_bridge::bridge_targets_language(bridge, &TARGET_SPELLINGS)
}

/// The configured bridges PHP actually emits, in configuration order.
pub fn active_bridges(config: &ResolvedCrateConfig) -> impl Iterator<Item = &TraitBridgeConfig> {
    config.trait_bridges_for(Language::Php)
}

pub(crate) fn reject_unsafe_bridges(config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    if let Some(bridge) = active_bridges(config).next() {
        anyhow::bail!(bridge.php_safety_error());
    }
    Ok(())
}

/// The trait a bridge wraps, when PHP emits that bridge at all.
///
/// PHP emits its consumer-facing wrapper class from `generate_public_api` and the `…Api`
/// extension class from `generate_bindings`, both of which forward to the `crate::<register_fn>`
/// item `gen_trait_bridge` writes — and that runs only when the trait resolves here. Both wrapper
/// passes ask this too, so an absent trait produces no wrapper rather than a wrapper calling a
/// symbol no pass generated. ~keep
pub fn active_bridge_trait<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}
