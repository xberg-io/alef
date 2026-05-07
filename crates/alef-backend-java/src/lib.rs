//! Java (Panama FFM) binding generator backend for alef.

mod gen_bindings;
pub mod gen_visitor;
mod template_env;
mod type_map;

pub use gen_bindings::JavaBackend;
