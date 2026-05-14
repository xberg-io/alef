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

/// Default output root when the workspace does not configure
/// `[crates.output].kotlin_android` explicitly.
const DEFAULT_AAR_ROOT: &str = "packages/kotlin-android";

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
        let aar_root = aar_root(config);

        let mut files = vec![
            GeneratedFile {
                path: aar_root.join("build.gradle.kts"),
                content: gen_build_gradle::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: aar_root.join("settings.gradle.kts"),
                content: gen_settings_gradle::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: aar_root.join("src/main/AndroidManifest.xml"),
                content: gen_manifest::emit(config),
                generated_header: false,
            },
            GeneratedFile {
                path: aar_root.join("consumer-rules.pro"),
                content: gen_proguard::emit_consumer(config),
                generated_header: false,
            },
            GeneratedFile {
                path: aar_root.join("proguard-rules.pro"),
                content: gen_proguard::emit_module(),
                generated_header: false,
            },
            GeneratedFile {
                path: aar_root.join(".gitignore"),
                content: gen_gitignore::emit(),
                generated_header: false,
            },
        ];

        files.extend(gen_jni_skeleton::emit(config, &aar_root));
        files.extend(gen_bindings::emit(api, config, &aar_root));
        files.extend(gen_java_facade::emit(api, config, &aar_root)?);

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

/// Resolved AAR project root. Falls back to [`DEFAULT_AAR_ROOT`] when the
/// workspace did not configure `[crates.output].kotlin_android`.
fn aar_root(config: &ResolvedCrateConfig) -> PathBuf {
    config
        .output_for("kotlin_android")
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AAR_ROOT))
}
