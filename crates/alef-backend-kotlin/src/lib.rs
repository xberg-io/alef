//! Kotlin (JVM) binding generator backend for alef.
//!
//! Phase 1A skeleton: registers `KotlinBackend` and exposes `BuildConfig`
//! with `BuildDependency::Ffi`. Kotlin/JVM consumes the same Java/Panama
//! `.so` produced by the Java backend; real codegen lands in Phase 1B.
//! Kotlin/Native and Kotlin Multiplatform paths are deferred to Phase 3.

mod gen_bindings;
mod gen_mpp;
pub(crate) mod gen_native;
mod type_map;

pub use gen_bindings::KotlinBackend;
