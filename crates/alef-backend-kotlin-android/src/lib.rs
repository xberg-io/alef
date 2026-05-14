//! Kotlin/Android (AAR library) backend for alef.
//!
//! Emits a self-contained Android library Gradle project:
//!
//! - `build.gradle.kts` with the Android Gradle Plugin and `maven-publish`
//! - `settings.gradle.kts` with `pluginManagement` so plugins resolve from a
//!   clean checkout
//! - `src/main/AndroidManifest.xml`
//! - `src/main/kotlin/<pkg>/<Module>.kt` — Kotlin facade wrapping the
//!   bundled Java facade, loading the native cdylib via
//!   `System.loadLibrary` on first class load
//! - `src/main/kotlin/<pkg>/DefaultClient.kt` — coroutine-friendly client
//!   class when the API has methodful types
//! - `src/main/java/<java_pkg>/*.java` — the full Java facade emitted by
//!   `alef-backend-java`, copied into the AAR so consumers do not need to
//!   depend on `packages/java/`
//! - `src/main/jniLibs/<abi>/.gitkeep` for each configured ABI (default
//!   `arm64-v8a`, `x86_64`)
//! - `consumer-rules.pro`, `proguard-rules.pro`, `.gitignore`
//!
//! Distinct from the JVM-only `alef-backend-kotlin` backend.

pub mod gen_bindings;
pub mod gen_build_gradle;
pub mod gen_gitignore;
pub mod gen_java_facade;
pub mod gen_jni_skeleton;
pub mod gen_manifest;
pub mod gen_proguard;
pub mod gen_settings_gradle;
pub mod naming;

use std::path::{Path, PathBuf};

use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;

use crate::naming::package_path;

/// Default output root when the workspace does not configure
/// `[crates.output].kotlin_android` explicitly.
const DEFAULT_AAR_ROOT: &str = "packages/kotlin-android";

/// Segment used by Gradle's Android source-set layout to separate the
/// project root from the Kotlin source destination
/// (`<project_root>/src/main/kotlin/<dotted_package>/`).
const KOTLIN_SOURCE_INFIX: &str = "src/main/kotlin";

/// Backend implementation for the Kotlin/Android target.
#[derive(Debug, Default, Clone, Copy)]
pub struct KotlinAndroidBackend;

impl Backend for KotlinAndroidBackend {
    fn name(&self) -> &str {
        "kotlin_android"
    }

    fn language(&self) -> Language {
        Language::KotlinAndroid
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: false,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let layout = ProjectLayout::resolve(config);

        let mut files = vec![
            GeneratedFile {
                path: layout.package_root.join("build.gradle.kts"),
                content: gen_build_gradle::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: layout.package_root.join("settings.gradle.kts"),
                content: gen_settings_gradle::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: layout.package_root.join("src/main/AndroidManifest.xml"),
                content: gen_manifest::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: layout.package_root.join("consumer-rules.pro"),
                content: gen_proguard::emit_consumer(config),
                generated_header: false,
            },
            GeneratedFile {
                path: layout.package_root.join("proguard-rules.pro"),
                content: gen_proguard::emit_module(),
                generated_header: false,
            },
            GeneratedFile {
                path: layout.package_root.join(".gitignore"),
                content: gen_gitignore::emit(),
                generated_header: false,
            },
        ];

        files.extend(gen_jni_skeleton::emit(config, &layout.package_root));
        files.extend(gen_bindings::emit(api, config, &layout.kotlin_source_dir));
        files.extend(gen_java_facade::emit(api, config, &layout.package_root)?);

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Resolved Android-AAR project paths.
///
/// `[crates.output].kotlin_android` semantically names the **Kotlin source
/// destination** — the directory that holds `<Module>.kt` and any Kotlin
/// facade files — because the Gradle Android source-set layout pins it to
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/`. The project
/// root (where `build.gradle.kts`, `AndroidManifest.xml`, `jniLibs/`, etc.
/// live) is derived by stripping that suffix.
///
/// When no output path is configured, the layout falls back to the legacy
/// default rooted at [`DEFAULT_AAR_ROOT`] and the Kotlin source dir is
/// computed from the package layout.
#[derive(Debug, Clone)]
struct ProjectLayout {
    /// Project root — where build metadata files (build.gradle.kts,
    /// settings.gradle.kts, AndroidManifest.xml, consumer/proguard rules,
    /// .gitignore, jniLibs/, src/main/java/) are emitted.
    package_root: PathBuf,
    /// Kotlin source destination — where `<Module>.kt` and Kotlin facade
    /// files are emitted.
    kotlin_source_dir: PathBuf,
}

impl ProjectLayout {
    fn resolve(config: &ResolvedCrateConfig) -> Self {
        let pkg_path = package_path(config);
        match config.output_for("kotlin_android") {
            Some(configured) => Self::from_configured(configured, &pkg_path),
            None => Self::default_for(&pkg_path),
        }
    }

    fn from_configured(configured: &Path, pkg_path: &str) -> Self {
        let kotlin_source_dir = configured.to_path_buf();
        let package_root = derive_package_root(configured, pkg_path);
        Self {
            package_root,
            kotlin_source_dir,
        }
    }

    fn default_for(pkg_path: &str) -> Self {
        let package_root = PathBuf::from(DEFAULT_AAR_ROOT);
        let kotlin_source_dir = package_root.join(KOTLIN_SOURCE_INFIX).join(pkg_path);
        Self {
            package_root,
            kotlin_source_dir,
        }
    }
}

/// Walk `configured` backwards to strip the `src/main/kotlin/<pkg_path>`
/// suffix. Falls back to treating the configured path as the project root
/// when the suffix cannot be matched — preserving the legacy semantics for
/// workspaces that point `kotlin_android` at the project root directly.
fn derive_package_root(configured: &Path, pkg_path: &str) -> PathBuf {
    let pkg_segment = PathBuf::from(pkg_path);
    let pkg_components: Vec<_> = pkg_segment.components().collect();
    let kotlin_components: Vec<_> = Path::new(KOTLIN_SOURCE_INFIX).components().collect();
    let configured_components: Vec<_> = configured.components().collect();

    let suffix_len = kotlin_components.len() + pkg_components.len();
    if configured_components.len() >= suffix_len {
        let tail_start = configured_components.len() - suffix_len;
        let tail = &configured_components[tail_start..];
        let kotlin_matches = tail[..kotlin_components.len()]
            .iter()
            .zip(kotlin_components.iter())
            .all(|(a, b)| a == b);
        let pkg_matches = tail[kotlin_components.len()..]
            .iter()
            .zip(pkg_components.iter())
            .all(|(a, b)| a == b);
        if kotlin_matches && pkg_matches {
            let head = &configured_components[..tail_start];
            if head.is_empty() {
                return PathBuf::from(".");
            }
            let mut root = PathBuf::new();
            for comp in head {
                root.push(comp);
            }
            return root;
        }
    }

    // Suffix did not match — treat configured path as the project root.
    configured.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_package_root_strips_kotlin_source_suffix() {
        let configured = Path::new("packages/kotlin-android/src/main/kotlin/dev/kreuzberg/kreuzcrawl/android");
        let root = derive_package_root(configured, "dev/kreuzberg/kreuzcrawl/android");
        assert_eq!(root, PathBuf::from("packages/kotlin-android"));
    }

    #[test]
    fn derive_package_root_falls_back_when_suffix_missing() {
        let configured = Path::new("packages/kotlin-android");
        let root = derive_package_root(configured, "dev/kreuzberg");
        assert_eq!(root, PathBuf::from("packages/kotlin-android"));
    }
}
