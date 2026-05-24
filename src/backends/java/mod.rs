//! Java (Panama FFM) binding generator backend for alef.

pub mod gen_bindings;
pub mod gen_visitor;
pub(crate) mod template_env;
pub(crate) mod type_map;

pub use gen_bindings::JavaBackend;
