//! Elixir (Rustler) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Elixir module-based callbacks via Rustler term dispatch.
//!
//! Two patterns are supported:
//!
//! 1. **Visitor bridge** (per-call, all methods have defaults): Accepts an Elixir map
//!    (`rustler::Term`) that encodes visitor overrides as function references
//!    (anonymous functions / `fn/arity` captures). Called via `rustler::Env::run_gc()`.
//!    Bridge param becomes `Option<rustler::Term<'_>>`.
//!
//! 2. **Plugin bridge** (registered, cached, async-friendly): Uses `LocalPid` to enable
//!    message passing to a GenServer-backed Elixir implementation. The bridge stores only
//!    a `LocalPid` (which is Copy + Send + Sync) and dispatches via channels to satisfy
//!    `Plugin: Send + Sync + 'static` bounds. Supports both sync (via `block_on`) and
//!    async dispatch to Elixir callbacks.

mod bridge_functions;
mod generator;
mod methods;
mod native_args;
#[cfg(test)]
mod tests;
mod visitor_bridge;

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
pub use bridge_functions::{gen_bridge_field_function, gen_bridge_function};
pub use generator::{RustlerBridgeGenerator, gen_trait_bridge};

use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};

/// The `exclude_languages` spellings that name this target: the language (`"elixir"`) and the
/// backend (`"rustler"`). Both are honoured so a consumer who names either one gets the same
/// answer from every site that asks. ~keep
pub const TARGET_SPELLINGS: [&str; 2] = ["elixir", "rustler"];

/// Whether `bridge` is emitted for the Elixir/Rustler target at all.
pub fn targets_rustler(bridge: &TraitBridgeConfig) -> bool {
    crate::codegen::generators::trait_bridge::bridge_targets_language(bridge, &TARGET_SPELLINGS)
}

/// The configured bridges Elixir/Rustler actually emits, in configuration order.
pub fn active_bridges(config: &ResolvedCrateConfig) -> impl Iterator<Item = &TraitBridgeConfig> {
    config.trait_bridges_for(Language::Elixir)
}

/// The trait a bridge wraps, when Elixir/Rustler emits that bridge at all.
///
/// The `defdelegate`-style register/unregister/clear wrappers in `public_api_delegates` call
/// `<AppModule>.Native.<fn>`, and those NIFs come from `native::gen_trait_bridge`, which runs
/// only when the trait resolves in the `ApiSurface`. Asking here keeps the Elixir module from
/// calling a NIF no pass generated. ~keep
pub fn active_bridge_trait<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}
