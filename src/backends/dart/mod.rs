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
    filter_excluded_functions, fix_handler_executor_calls, inject_display_as_text_methods,
    make_struct_fields_with_defaults_optional, rewrite_frb_sealed_variants, strip_trailing_whitespace,
};
pub use gen_bindings::DartBackend;
