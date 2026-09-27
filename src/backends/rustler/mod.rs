//! Elixir (Rustler) binding generator backend for alef.

mod elixir_escape;
pub(crate) mod gen_bindings;
pub(crate) mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::RustlerBackend;
