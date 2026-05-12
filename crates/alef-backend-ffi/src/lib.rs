//! C FFI binding generator backend for alef.

mod gen_bindings;
pub mod gen_bridge_field;
mod gen_visitor;
pub(crate) mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::FfiBackend;
