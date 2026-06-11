//! Naming helpers for the Kotlin/Android backend.
//!
//! Centralises every defaulting rule that derives a name (package, namespace,
//! Maven artifact id, ABI list, JVM target, etc.) from the
//! [`KotlinAndroidConfig`][crate::core::config::languages::KotlinAndroidConfig]
//! plus the crate name. The backend itself never reads the raw config — it
//! always goes through these helpers so a single rule change here propagates
//! to every emitted file.

use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions::toolchain;

/// The JVM-style Kotlin package for the emitted bindings (e.g.
/// `dev.sample_core`). Falls back to a sanitized crate name.
pub fn kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.package.clone())
        .unwrap_or_else(|| sanitize_package(&config.name))
}

/// The Android library manifest `namespace`. Defaults to [`kotlin_package`].
pub fn namespace(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.namespace.clone())
        .unwrap_or_else(|| kotlin_package(config))
}

/// The Maven `artifactId` for the generated AAR. Defaults to
/// `{crate}-android`.
pub fn aar_artifact_id(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.artifact_id.clone())
        .unwrap_or_else(|| format!("{}-android", config.name.replace('_', "-")))
}

/// The Maven `groupId` for the generated AAR. Falls back to the Kotlin
/// package when unset (a sensible default for repos that publish under their
/// own reverse-DNS namespace).
pub fn aar_group_id(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.group_id.clone())
        .unwrap_or_else(|| kotlin_package(config))
}

/// Path-style version of [`kotlin_package`] (`dev.sample_core` →
/// `dev/sample_core`).
pub fn package_path(config: &ResolvedCrateConfig) -> String {
    kotlin_package(config).replace('.', "/")
}

/// JVM-style package for the bundled Java facade. Re-uses the existing
/// `java_package()` accessor on the resolved config so the Java backend's
/// behaviour stays consistent.
pub fn java_package(config: &ResolvedCrateConfig) -> String {
    config.java_package()
}

/// Path-style version of [`java_package`].
pub fn java_package_path(config: &ResolvedCrateConfig) -> String {
    java_package(config).replace('.', "/")
}

/// Android `compileSdk` level. Defaults to
/// `toolchain::ANDROID_COMPILE_SDK`.
pub fn compile_sdk(config: &ResolvedCrateConfig) -> u32 {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.compile_sdk)
        .unwrap_or_else(|| {
            toolchain::ANDROID_COMPILE_SDK
                .parse()
                .expect("ANDROID_COMPILE_SDK must parse as u32")
        })
}

/// Android `minSdk` level. Defaults to `toolchain::ANDROID_MIN_SDK`.
pub fn min_sdk(config: &ResolvedCrateConfig) -> u32 {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.min_sdk)
        .unwrap_or_else(|| {
            toolchain::ANDROID_MIN_SDK
                .parse()
                .expect("ANDROID_MIN_SDK must parse as u32")
        })
}

/// JVM bytecode target used for both Kotlin and Java compilation
/// (e.g. `"17"`). Defaults to `toolchain::ANDROID_JVM_TARGET`.
pub fn jvm_target(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.jvm_target.clone())
        .unwrap_or_else(|| toolchain::ANDROID_JVM_TARGET.to_string())
}

/// ABI directories scaffolded under `src/main/jniLibs/<abi>/`.
pub fn abis(config: &ResolvedCrateConfig) -> Vec<String> {
    config
        .kotlin_android
        .as_ref()
        .and_then(|c| c.abis.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["arm64-v8a".to_string(), "x86_64".to_string()])
}

/// Host platform directory name based on the target OS.
/// Returns "darwin", "linux", or "windows".
pub fn host_platform_dir() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// All supported host platform names for emitting test resource placeholders.
pub const HOST_PLATFORMS: &[&str] = &["darwin", "linux", "windows"];

/// Return the canonical Kotlin Android bridge object name for a trait.
///
/// Both the production wrapper codegen (`trait_bridge.rs`) and the e2e stub
/// emitter (`e2e/codegen/kotlin_android.rs`) must call this function so that a
/// rename in one place is automatically reflected in the other.
///
/// # Example
/// ```
/// use alef::backends::kotlin_android::naming::bridge_object_name;
/// assert_eq!(bridge_object_name("OcrBackend"), "OcrBackendBridge");
/// ```
pub fn bridge_object_name(trait_name: &str) -> String {
    format!("{trait_name}Bridge")
}

fn sanitize_package(name: &str) -> String {
    name.replace('-', "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect()
}
