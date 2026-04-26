//! Go (cgo) binding generator backend for alef.

mod gen_bindings;
pub mod gen_visitor;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::GoBackend;
