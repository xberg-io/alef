//! Dart binding generator backend for alef.
//!
//! Two bridging styles are supported:
//!
//! - **FRB** (`style = "frb"`, default): emits a Rust crate plus Dart wrappers using
//!   flutter_rust_bridge-generated bridge symbols (Phase 2).
//! - **FFI** (`style = "ffi"`): emits Dart-only `dart:ffi` source that loads the
//!   cbindgen-produced C shared library directly at runtime (Phase 3). No Rust crate
//!   is generated; the same C library consumed by Go/Java/C#/Zig is reused.

mod gen_bindings;
pub(crate) mod gen_ffi;
pub(crate) mod gen_rust_crate;
mod type_map;

pub use gen_bindings::DartBackend;
