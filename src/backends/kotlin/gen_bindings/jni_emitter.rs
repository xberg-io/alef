//! JNI emission mode for the Kotlin backend.
//!
//! When `[crates.kotlin] ffi_style = "jni"` (or when forced by the Android
//! backend), this module emits:
//!
//! - `<Module>Bridge.kt` — a Kotlin `object` with `external fun` declarations
//!   and an `init { System.loadLibrary("<crate>_jni") }` block.
//! - `DefaultClient.kt` — a Kotlin class holding a `Long` handle that delegates
//!   every method to the Bridge object via JNI. Streaming methods use the same
//!   `callbackFlow` pattern as the Panama path but reference `handle: Long`
//!   instead of `inner: <JavaFacadeType>`.
//!
//! No `java.lang.foreign.*` imports are emitted anywhere in this module.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::core::backend::GeneratedFile;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

use super::object_wrapper::{format_param_with_imports, kotlin_type_with_string_imports};
use super::shared::{to_lower_camel, to_pascal_case};
use crate::backends::kotlin::template_env;

include!("jni_emitter/bridge_object.rs");
include!("jni_emitter/external_functions.rs");
include!("jni_emitter/client_class.rs");
include!("jni_emitter/client_methods.rs");
include!("jni_emitter/constructors.rs");
include!("jni_emitter/binary_json.rs");
include!("jni_emitter/trait_bridge.rs");
include!("jni_emitter/paths.rs");
// Included last so the `#[cfg(test)]` module is the final item in this flattened module
// (`clippy::items_after_test_module`).
include!("jni_emitter/tests.rs");
