//! WebAssembly (wasm-bindgen) binding generator backend for alef.
//!
//! Generates JavaScript-compatible WebAssembly bindings using the wasm-bindgen crate.
//! Supports configurable restriction handling through `WasmConfig`:
//! - `exclude_functions`: Skip generation of specific functions
//! - `exclude_types`: Skip generation of specific types
//! - `type_overrides`: Remap types (e.g., Path → String)

pub(crate) mod gen_bindings;
mod template_env;
pub mod trait_bridge;
pub(crate) mod type_map;

#[cfg(test)]
mod wasm_bindgen_js_oracle;

pub use gen_bindings::WasmBackend;
pub(crate) use gen_bindings::{WasmCallability, docs_ts_type_for_untagged_enum, wasm_callability_with_excluded_types};
