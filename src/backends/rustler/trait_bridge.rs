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
