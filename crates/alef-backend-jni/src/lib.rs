//! Rust JNI shim emitter backend for alef.
//!
//! Emits a single `lib.rs` file into the consumer's `<crate>-jni` Rust crate
//! that exports `Java_*` symbols matching every `external fun native*` declared
//! in the paired `alef-backend-kotlin-android` output.
//!
//! The emitted file:
//! - `#![allow(non_snake_case)]` — JNI symbol names violate Rust naming conventions.
//! - One `pub unsafe extern "system" fn Java_<pkg>_<Bridge>_<method>` per API
//!   function and per instance method on every opaque client type.
//! - Streaming adapter shims: `..._Start`, `..._Next`, `..._Free`.
//! - `jni` crate used at the JNI boundary; the consumer's `Cargo.toml` must
//!   declare `jni` as a dependency.
//!
//! This backend is **not** registered in the alef-cli dispatch table as a
//! standalone language target. It is driven by `Language::Jni` which is always
//! paired with `Language::KotlinAndroid`. The [`JniBackend`] struct is exported
//! for direct use by the cli pipeline via the `Language::Jni` arm.

mod gen_shims;

pub use gen_shims::JniBackend;
