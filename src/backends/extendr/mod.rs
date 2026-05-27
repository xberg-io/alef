//! R (extendr) binding generator backend for alef.

pub mod gen_bindings;
pub(crate) mod template_env;
pub mod trait_bridge;

pub use gen_bindings::ExtendrBackend;
