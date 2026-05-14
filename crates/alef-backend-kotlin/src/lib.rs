//! Kotlin (JVM) binding generator backend for alef.
//!
//! Phase 1A skeleton: registers `KotlinBackend` and exposes `BuildConfig`
//! with `BuildDependency::Ffi`. Kotlin/JVM consumes the same Java/Panama
//! `.so` produced by the Java backend; real codegen lands in Phase 1B.
//! Kotlin/Native and Kotlin Multiplatform paths are deferred to Phase 3.

mod gen_bindings;
mod gen_mpp;
pub(crate) mod gen_native;
pub mod naming;
mod template_env;
mod type_map;

pub use gen_bindings::KotlinBackend;
pub use gen_bindings::trait_bridge::KotlinJvmBridgeGenerator;

// Re-exports used by the sibling `alef-backend-kotlin-android` crate so it can
// emit the same Kotlin/JVM-flavoured glue code for AAR consumers without
// duplicating the helpers.
pub use gen_bindings::{
    emit_enum_pub, emit_error_type_pub, emit_function_jvm, emit_jvm_client_class, emit_type_pub, to_pascal_case,
};
