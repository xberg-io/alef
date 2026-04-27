//! Zig binding generator backend for alef.
//!
//! Phase 1A skeleton: registers `ZigBackend` and exposes `BuildConfig`
//! with `BuildDependency::Ffi`. Real codegen lands in Phase 1B.

mod gen_bindings;
mod trait_bridge;
mod type_map;

pub use gen_bindings::ZigBackend;
