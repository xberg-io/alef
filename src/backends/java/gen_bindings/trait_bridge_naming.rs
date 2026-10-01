//! Names of the Java trait-bridge registration API.
//!
//! `trait_bridge.rs` emits the class and its methods; `JavaBackend::trait_bridge_registration_
//! surface` reports them to the reference-doc renderer. Both spell the names from here so the
//! docs cannot describe a method the emitted class does not declare.

use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;

/// PascalCase form of a trait name, as every emitted Java identifier derived from it spells it.
pub(crate) fn trait_pascal(trait_name: &str) -> String {
    trait_name.to_pascal_case()
}

/// One abstract method the Java plugin lifecycle contract always requires.
pub(crate) struct SuperTraitMethod {
    /// The snake_case method name, matching the interface's declared signature verbatim.
    pub(crate) name: &'static str,
}

/// Abstract methods `trait_interface.jinja`'s `{% if has_super_trait %}` block always declares
/// on `I<Trait>` when a bridge configures a `super_trait` -- `initialize()`/`shutdown()` are
/// emitted as `default` methods there and need no override, so they are not part of this list.
///
/// The Java plugin lifecycle is a fixed host-language convention, not something read off the
/// real Rust super-trait: `gen_interface_file` emits it unconditionally from `has_super_trait`
/// alone and never looks up the super-trait's `TypeDef`. The e2e trait-bridge stub generator
/// (`e2e::codegen::java::args::build_args_and_setup`) used to be the one place that *did* look
/// it up -- by matching `TraitBridgeConfig::super_trait` against `TypeDef::rust_path` -- and
/// silently produced no methods at all when that lookup missed (e.g. a super-trait declared in
/// a private module and re-exported via `pub use`, whose `rust_path` does not necessarily equal
/// the configured value). The interface still required `name()`/`version()` either way, so the
/// stub was `not abstract and does not override abstract method version()`. Read this constant
/// wherever that guarantee is needed instead of re-deriving it from the super-trait's IR. ~keep
pub(crate) const SUPER_TRAIT_REQUIRED_METHODS: [SuperTraitMethod; 2] =
    [SuperTraitMethod { name: "name" }, SuperTraitMethod { name: "version" }];

/// The static class holding the registration methods, e.g. `SamplePluginBridge`.
pub(crate) fn bridge_class_name(trait_pascal: &str) -> String {
    format!("{trait_pascal}Bridge")
}

/// `register<Trait>` — always emitted, and named after the trait rather than the configured
/// `register_fn`.
pub(crate) fn register_method_name(trait_pascal: &str) -> String {
    format!("register{trait_pascal}")
}

/// `unregister<Trait>` — emitted only when `unregister_fn` is configured, but named after the
/// trait, not after that configured value.
pub(crate) fn unregister_method_name(trait_pascal: &str) -> String {
    format!("unregister{trait_pascal}")
}

/// `clear<Plural>` — the one registration method whose name *is* derived from its configured
/// value, so that `clear_text_backends` becomes `clearTextBackends` and keeps the plural the
/// Rust side spells. The `clear_` prefix is re-attached rather than camel-cased along with the
/// rest, so a `clear_fn` that does not start with `clear_` still yields a `clear`-prefixed
/// method. ~keep
pub(crate) fn clear_method_name(clear_fn: &str) -> String {
    let without_prefix = clear_fn.strip_prefix("clear_").unwrap_or(clear_fn);
    let mut camel = String::from("clear");
    for word in without_prefix.split('_') {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            camel.extend(first.to_uppercase());
            camel.push_str(chars.as_str());
        }
    }
    camel
}

/// The registration API `gen_trait_bridge_files` emits for each configured bridge.
///
/// Mirrors the gates `JavaBackend::generate_bindings` applies before calling it: the bridge must
/// not exclude `java`, an `options_field` bridge is handled by the visitor path instead, and the
/// trait must resolve in the `ApiSurface`. The visitor gate is only active when the crate has a
/// visitor pattern, matching the emitter. ~keep
pub(crate) fn registration_surface(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    has_visitor_pattern: bool,
) -> Vec<TraitBridgeRegistrationSurface> {
    config
        .trait_bridges_for(Language::Java)
        .filter(|bridge| !(has_visitor_pattern && bridge.bind_via == BridgeBinding::OptionsField))
        .filter_map(|bridge| {
            let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name && t.is_trait)?;
            let pascal = trait_pascal(&trait_def.name);
            let bridge_class = bridge_class_name(&pascal);
            Some(TraitBridgeRegistrationSurface {
                trait_name: trait_def.name.clone(),
                register_symbol: Some(format!("{bridge_class}.{}", register_method_name(&pascal))),
                unregister_symbol: bridge
                    .unregister_fn
                    .as_ref()
                    .map(|_| format!("{bridge_class}.{}", unregister_method_name(&pascal))),
                clear_symbol: bridge
                    .clear_fn
                    .as_deref()
                    .map(|clear_fn| format!("{bridge_class}.{}", clear_method_name(clear_fn))),
            })
        })
        .collect()
}
