//! PHP (ext-php-rs) binding generator backend for alef.

mod gen_bindings;
pub mod naming;
mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::PhpBackend;
