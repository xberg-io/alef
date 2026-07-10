use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// FFI strategy for Kotlin JVM / Android binding emission.
///
/// - `"panama"` (default): consumes the Java/Panama FFM facade emitted by
///   `alef-backend-java`. Requires JDK 22+ at runtime. Not supported on
///   Android Runtime.
/// - `"jni"`: emits a `object <Module>Bridge { external fun native<...>(...) }`
///   object with JNI declarations and a `DefaultClient` class holding a `Long`
///   handle. Compatible with Android Runtime (JDK 11). Consumers must ship a
///   `<crate>-jni` Rust crate exporting matching `Java_*` JNI symbols.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KotlinFfiStyle {
    #[default]
    Panama,
    Jni,
}

/// Target platform for Kotlin code generation.
///
/// - `"jvm"` (default): emits source consuming the Java/Panama FFM facade.
/// - `"native"`: emits Kotlin/Native source consuming the cbindgen C FFI library.
/// - `"multiplatform"`: emits Kotlin Multiplatform project scaffolding.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KotlinTarget {
    #[default]
    Jvm,
    Native,
    Multiplatform,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct KotlinConfig {
    pub package: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Kotlin binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Kotlin binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Target platform for Kotlin output. `"jvm"` (default) emits source consuming
    /// the Java/Panama FFM facade; `"native"` emits Kotlin/Native source consuming
    /// the cbindgen C FFI library. `"multiplatform"` emits KMP scaffolding.
    #[serde(default)]
    pub target: KotlinTarget,
    /// Emission mode controlling which Kotlin project layout is generated.
    ///
    /// Accepted values:
    /// - `"jvm"` (default) — standard JVM-only project under `packages/kotlin/`
    /// - `"kmp"` — Kotlin Multiplatform project under `packages/kotlin-mpp/`
    /// - `"android"` — Android library project under `packages/kotlin-android/`
    ///
    /// When `None`, defaults to `"jvm"`.
    #[serde(default)]
    pub mode: Option<String>,
    /// FFI strategy. `"panama"` (default) consumes the Java/Panama FFM facade.
    /// `"jni"` emits a Kotlin Bridge object with `external fun` declarations
    /// and a `DefaultClient` class holding a `Long` handle. Android backend
    /// forces `"jni"` regardless of this setting.
    #[serde(default)]
    pub ffi_style: KotlinFfiStyle,
}
