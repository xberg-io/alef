//! Python (PyO3) binding generator backend for alef.

mod gen_bindings;
mod gen_stubs;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::Pyo3Backend;
