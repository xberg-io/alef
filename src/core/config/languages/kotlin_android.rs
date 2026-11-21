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
#[serde(deny_unknown_fields)]
pub struct KotlinAndroidConfig {
    /// Map of Rust type name -> host-native capsule (Language-passthrough) config.
    /// When set, functions returning the type construct the host runtime's native
    /// `Language` (e.g. ktreesitter's `io.github.treesitter.ktreesitter.Language` from a
    /// native `long` pointer) instead of an opaque handle.
    /// See [`crate::core::config::HostCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    /// Affirms every configured capsule wrapper uses the exact native runtime instance ~keep
    /// and ownership contract that produced its pointer. ~keep
    #[serde(default)]
    pub shares_native_runtime: bool,
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
    /// Files bundled into the AAR under `META-INF/`, as paths relative to the workspace root
    /// (e.g. `["LICENSE", "THIRD_PARTY_LICENSES.md"]`). Each is staged by a generated Gradle
    /// task and added to the main source set's resources; the build fails if one is missing.
    #[serde(default)]
    pub bundled_notices: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_native_runtime_is_accepted_from_toml() {
        let parsed = toml::from_str::<KotlinAndroidConfig>("shares_native_runtime = true");
        let config = parsed.unwrap_or_else(|error| {
            panic!(
                "the capsule gate tells users to set `[crates.kotlin_android].shares_native_runtime = true`, \
                 so that key must parse; got: {error}"
            )
        });
        assert!(config.shares_native_runtime);
    }

    #[test]
    fn bundled_notices_parse_from_toml_and_default_to_empty() {
        let config =
            toml::from_str::<KotlinAndroidConfig>(r#"bundled_notices = ["LICENSE", "THIRD_PARTY_LICENSES.md"]"#)
                .expect("bundled_notices parses");
        assert_eq!(config.bundled_notices, ["LICENSE", "THIRD_PARTY_LICENSES.md"]);
        assert!(
            toml::from_str::<KotlinAndroidConfig>("")
                .unwrap()
                .bundled_notices
                .is_empty()
        );
    }

    #[test]
    fn shares_native_runtime_defaults_to_false() {
        let config = toml::from_str::<KotlinAndroidConfig>("").expect("empty config parses");
        assert!(!config.shares_native_runtime);
    }
}
