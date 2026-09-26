//! Dart binding generator backend for alef.
//!
//! Two bridging styles are supported:
//!
//! - **FRB** (`style = "frb"`, default): emits a Rust crate plus Dart wrappers using
//!   flutter_rust_bridge-generated bridge symbols (Phase 2).
//! - **FFI** (`style = "ffi"`): emits Dart-only `dart:ffi` source that loads the
//!   cbindgen-produced C shared library directly at runtime (Phase 3). No Rust crate
//!   is generated; the same C library consumed by Go/Java/C#/Zig is reused.

mod frb_rewrite;
mod gen_bindings;
pub(crate) mod gen_ffi;
pub(crate) mod gen_rust_crate;
pub(crate) mod ident;
pub mod naming;
mod template_env;
pub(crate) mod type_map;

#[cfg(test)]
mod plugin_trait_stubs_test;

pub use frb_rewrite::{
    carry_lib_rs_cfg_gates_into_frb_generated, cfg_gated_free_functions, filter_excluded_functions,
    fix_handler_executor_calls, inject_display_as_text_methods, inject_frb_cfg_gates,
    make_struct_fields_with_defaults_optional, missing_bridge_functions, missing_bridge_struct_fields,
    rewrite_frb_sealed_variants, strip_trailing_whitespace, undeclared_gate_features,
};
pub use gen_bindings::DartBackend;
pub(crate) use gen_bindings::{config_param_is_named_optional, frb_dart_bridge_path, frb_rust_facade_paths};
