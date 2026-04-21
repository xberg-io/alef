//! PHP (ext-php-rs) binding generator backend for alef.

mod gen_bindings;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::PhpBackend;
