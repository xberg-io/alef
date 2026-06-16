//! JNI shim code generator.
//!
//! Emits a single `lib.rs` into the consumer's `<crate>-jni` Rust crate.  The
//! emitted file exports `pub unsafe extern "system" fn Java_*` symbols that
//! satisfy every `external fun native*` declaration produced by
//! `alef-backend-kotlin-android`.
//!
//! # Symbol naming — JNI spec §5.11.3
//!
//! `Java_<package_underscored>_<Class>_<method>`
//!
//! Underscores inside any identifier segment are encoded as `_1`.  Package
//! dots become `_`.  The helpers in [`crate::core::jni`] own the canonical
//! encoding so this backend and the Kotlin backend can never drift apart.

use std::path::PathBuf;

use minijinja::context;

use crate::backends::jni::template_env;
use crate::codegen::generators::collect_trait_imports;
use crate::codegen::naming::to_class_name;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, ParamDef, PrimitiveType, TypeDef, TypeRef};
use crate::core::jni::{
    bridge_class_name, bridge_method_name, destructor_method_name, jni_symbol, streaming_method_names,
};

include!("gen_shims/backend.rs");
include!("gen_shims/top_level.rs");
include!("gen_shims/trait_registration.rs");
include!("gen_shims/runtime_helpers.rs");
include!("gen_shims/client_shims.rs");
include!("gen_shims/marshalling.rs");
include!("gen_shims/function_shims.rs");
include!("gen_shims/method_shims.rs");
include!("gen_shims/constructor_shims.rs");
include!("gen_shims/streaming_shims.rs");
include!("gen_shims/type_helpers.rs");
include!("gen_shims/tests.rs");
