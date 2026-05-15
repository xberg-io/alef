//! Kotlin/Android (AAR library) backend for alef.
//!
//! Emits a self-contained Android library Gradle project with a pure-Kotlin
//! JNI layout — no bundled Java facade. All binding code lives under
//! `src/main/kotlin/`.
//!
//! - `build.gradle.kts` with the Android Gradle Plugin and `maven-publish`
//! - `settings.gradle.kts` with `pluginManagement` so plugins resolve from a
//!   clean checkout
//! - `src/main/AndroidManifest.xml`
//! - `src/main/kotlin/<pkg>/<Module>Bridge.kt` — a Kotlin `object` with
//!   `external fun` JNI declarations and `init { System.loadLibrary(...) }`
//! - `src/main/kotlin/<pkg>/DefaultClient.kt` — coroutine-friendly client
//!   class holding a `Long` handle when the API has methodful types
//! - `src/main/jniLibs/<abi>/.gitkeep` for each configured ABI (default
//!   `arm64-v8a`, `x86_64`)
//! - `consumer-rules.pro`, `proguard-rules.pro`, `.gitignore`
//!
//! Forces `KotlinFfiStyle::Jni` regardless of the workspace configuration.
//! Consumers must ship a `<crate>-jni` Rust crate exporting
//! `Java_<package>_<Module>Bridge_native<Method>` symbols per JNI spec §5.11.3
//! and link `lib<crate>_jni.so` into `jniLibs/<abi>/`.
//!
//! Distinct from the JVM-only `alef-backend-kotlin` backend.

pub mod gen_bindings;
pub mod gen_build_gradle;
pub mod gen_editorconfig;
pub mod gen_gitignore;
pub mod gen_jni_skeleton;
pub mod gen_manifest;
pub mod gen_proguard;
pub mod gen_settings_gradle;
pub mod naming;

use std::path::{Path, PathBuf};

use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{KotlinFfiStyle, Language, ResolvedCrateConfig};
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
            supports_streaming: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Always force JNI mode: the Android AAR does not ship a Java/Panama facade.
        let config = config.clone().with_kotlin_ffi_style(KotlinFfiStyle::Jni);
        let config = &config;
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
            GeneratedFile {
                path: layout.package_root.join(".editorconfig"),
                content: gen_editorconfig::emit(),
                generated_header: false,
            },
        ];

        files.extend(gen_jni_skeleton::emit(config, &layout.package_root));
        files.extend(gen_bindings::emit(api, config, &layout.kotlin_source_dir));

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
            None => Self::rooted_at(&PathBuf::from(DEFAULT_AAR_ROOT), &pkg_path),
        }
    }

    /// Interpret a configured `[crates.output].kotlin_android` path.
    ///
    /// When the path ends with the Gradle Android source-set suffix
    /// `src/main/kotlin/<dotted_package_as_path>/`, the configured path
    /// is the Kotlin source destination and the project root is the
    /// prefix before that suffix.
    ///
    /// Otherwise, fall back to treating the configured path as the
    /// project root (legacy semantics — preserves behaviour for
    /// workspaces and the workspace template default that point
    /// `kotlin_android` at the project root directly).
    fn from_configured(configured: &Path, pkg_path: &str) -> Self {
        if let Some(package_root) = strip_kotlin_source_suffix(configured, pkg_path) {
            Self {
                package_root,
                kotlin_source_dir: configured.to_path_buf(),
            }
        } else {
            Self::rooted_at(configured, pkg_path)
        }
    }

    /// Compose a layout rooted at `package_root` with the Kotlin source
    /// destination derived from the Gradle Android source-set layout.
    fn rooted_at(package_root: &Path, pkg_path: &str) -> Self {
        Self {
            package_root: package_root.to_path_buf(),
            kotlin_source_dir: package_root.join(KOTLIN_SOURCE_INFIX).join(pkg_path),
        }
    }
}

/// Walk `configured` backwards to strip the `src/main/kotlin/<pkg_path>`
/// suffix. Returns the project-root prefix on a match, or `None` when the
/// suffix is absent.
fn strip_kotlin_source_suffix(configured: &Path, pkg_path: &str) -> Option<PathBuf> {
    let pkg_segment = PathBuf::from(pkg_path);
    let pkg_components: Vec<_> = pkg_segment.components().collect();
    let kotlin_components: Vec<_> = Path::new(KOTLIN_SOURCE_INFIX).components().collect();
    let configured_components: Vec<_> = configured.components().collect();

    let suffix_len = kotlin_components.len() + pkg_components.len();
    if configured_components.len() < suffix_len {
        return None;
    }
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
    if !(kotlin_matches && pkg_matches) {
        return None;
    }
    let head = &configured_components[..tail_start];
    if head.is_empty() {
        return Some(PathBuf::from("."));
    }
    let mut root = PathBuf::new();
    for comp in head {
        root.push(comp);
    }
    Some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_kotlin_source_suffix_extracts_project_root() {
        let configured = Path::new("packages/kotlin-android/src/main/kotlin/dev/kreuzberg/kreuzcrawl/android");
        let root = strip_kotlin_source_suffix(configured, "dev/kreuzberg/kreuzcrawl/android");
        assert_eq!(root, Some(PathBuf::from("packages/kotlin-android")));
    }

    #[test]
    fn strip_kotlin_source_suffix_returns_none_when_suffix_missing() {
        let configured = Path::new("packages/kotlin-android");
        assert_eq!(strip_kotlin_source_suffix(configured, "dev/kreuzberg"), None);
    }

    #[test]
    fn from_configured_derives_package_root_when_path_targets_kotlin_source() {
        let configured = Path::new("packages/kotlin-android/src/main/kotlin/dev/kreuzberg/kreuzcrawl/android");
        let layout = ProjectLayout::from_configured(configured, "dev/kreuzberg/kreuzcrawl/android");
        assert_eq!(layout.package_root, PathBuf::from("packages/kotlin-android"));
        assert_eq!(layout.kotlin_source_dir, PathBuf::from(configured));
    }

    #[test]
    fn from_configured_falls_back_to_legacy_when_path_is_project_root() {
        let configured = Path::new("packages/kotlin-android");
        let layout = ProjectLayout::from_configured(configured, "dev/kreuzberg");
        assert_eq!(layout.package_root, PathBuf::from("packages/kotlin-android"));
        assert_eq!(
            layout.kotlin_source_dir,
            PathBuf::from("packages/kotlin-android/src/main/kotlin/dev/kreuzberg")
        );
    }
}
