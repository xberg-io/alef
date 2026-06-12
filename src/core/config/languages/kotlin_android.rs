use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the dedicated Kotlin/Android backend (`alef-backend-kotlin-android`).
///
/// Distinct from [`crate::core::config::languages::KotlinConfig`] (Kotlin/JVM). When a crate targets the
/// `kotlin_android` language slug, this struct controls the emitted
/// `build.gradle.kts`, `AndroidManifest.xml`, namespace, Maven publish
/// coordinates, ABI list, and the bundled Java facade emitted into
/// `src/main/java/` so the AAR is self-contained.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct KotlinAndroidConfig {
    /// JVM-style package for Kotlin bindings (e.g. `dev.sample_core`).
    /// Defaults to the crate name.
    #[serde(default)]
    pub package: Option<String>,
    /// Android library manifest `namespace`. Defaults to `package`.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Maven `artifactId` for the generated AAR. Defaults to `{crate}-android`.
    #[serde(default)]
    pub artifact_id: Option<String>,
    /// Maven `groupId` for the generated AAR. No default — when unset the
    /// emitter falls back to `package`.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Android compile SDK level. Defaults to `template_versions::toolchain::ANDROID_COMPILE_SDK`.
    #[serde(default)]
    pub compile_sdk: Option<u32>,
    /// Android min SDK level. Defaults to `template_versions::toolchain::ANDROID_MIN_SDK`.
    #[serde(default)]
    pub min_sdk: Option<u32>,
    /// JVM bytecode target for Kotlin and Java compilation
    /// (e.g. `"17"`). Defaults to `template_versions::toolchain::ANDROID_JVM_TARGET`.
    #[serde(default)]
    pub jvm_target: Option<String>,
    /// ABIs to scaffold under `src/main/jniLibs/<abi>/`. Defaults to
    /// `["arm64-v8a", "x86_64"]`.
    #[serde(default)]
    pub abis: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
}
