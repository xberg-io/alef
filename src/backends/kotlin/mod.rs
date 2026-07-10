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
pub(crate) mod type_map;

pub use gen_bindings::KotlinBackend;
pub use gen_bindings::trait_bridge::KotlinJvmBridgeGenerator;

pub use gen_bindings::{
    emit_enum_pub, emit_error_type_pub, emit_function_jvm, emit_jvm_client_class, emit_jvm_client_class_with_package,
    emit_kdoc_pub, emit_type_pub, emit_type_pub_with_defaults_sealed_and_constructible,
    emit_type_pub_with_enum_defaults, emit_type_pub_with_enum_defaults_and_sealed_classes, kotlin_type_str_pub,
    to_lower_camel, to_pascal_case,
};

pub use gen_bindings::jni_emitter::{emit_jni_bridge_object, emit_jni_client_class, emit_streaming_jni_external_funs};

pub use gen_bindings::literal_normalizer;
