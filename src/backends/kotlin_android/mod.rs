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
//! `Java_<package>_<Module>Bridge_native<Method>` symbols per JNI spec §5.11.3.
//! The emitted `buildAndroidJniLibs` gradle task cross-compiles that crate with
//! `cargo-ndk` into `jniLibs/<abi>/` for release and publish task graphs; a
//! workflow that stages prebuilt libraries there itself is detected and left
//! alone, and `-Palef.skipAndroidJni=true` disables the task outright.
//!
//! Distinct from the JVM-only `alef-backend-kotlin` backend.

pub mod gen_bindings;
pub mod gen_build_gradle;
pub mod gen_editorconfig;
pub mod gen_gitignore;
pub mod gen_gradle_properties;
pub mod gen_jni_skeleton;
pub mod gen_manifest;
pub mod gen_proguard;
pub mod gen_seed_test;
pub mod gen_settings_gradle;
pub mod gradle_wrapper;
pub mod naming;
pub mod template_env;
pub mod trait_bridge;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::backends::kotlin::literal_normalizer;
use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, TraitBridgeRegistrationSurface,
};
use crate::core::config::{KotlinFfiStyle, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

use crate::backends::kotlin_android::naming::package_path;

/// Collect all type names excluded for the `kotlin_android` language from both
/// the per-language `[crates.kotlin_android].exclude_types` list and the shared
/// `[crates.ffi].exclude_types` list (mirroring the Java backend pattern).
fn effective_exclude_types(config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ka) = &config.kotlin_android {
        exclude_types.extend(ka.exclude_types.iter().cloned());
    }
    exclude_types
}

/// Return true when `ty` references any type name in `exclude_types`.
fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

/// Return true when any parameter type or the return type references an
/// excluded type name.
fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

/// Build a filtered copy of `api` with all excluded types (and any
/// fields / methods / functions that reference them) removed.
fn api_without_excluded_types(api: &ApiSurface, exclude_types: &HashSet<String>) -> ApiSurface {
    let mut filtered = api.clone();
    filtered.types.retain(|typ| !exclude_types.contains(&typ.name));
    for typ in &mut filtered.types {
        typ.fields
            .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        typ.methods
            .retain(|method| !signature_references_excluded_type(&method.params, &method.return_type, exclude_types));
    }
    filtered
        .enums
        .retain(|enum_def| !exclude_types.contains(&enum_def.name));
    for enum_def in &mut filtered.enums {
        for variant in &mut enum_def.variants {
            variant
                .fields
                .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        }
    }
    filtered
        .functions
        .retain(|func| !signature_references_excluded_type(&func.params, &func.return_type, exclude_types));
    filtered.errors.retain(|error| !exclude_types.contains(&error.name));
    filtered
}

/// Default output root when the workspace does not configure
/// `[crates.output].kotlin_android` explicitly.
///
/// [`crate::core::config::ResolvedCrateConfig::package_dir`] reads this rather than repeating the
/// literal, so the directory the build command `cd`s into and the one this backend documents as
/// its default cannot drift apart. ~keep
pub const DEFAULT_AAR_ROOT: &str = "packages/kotlin-android";

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
            supports_service_api: false,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        let config = config.clone().with_kotlin_ffi_style(KotlinFfiStyle::Jni);
        let config = &config;
        validate_capsule_ffi_parity(config)?;
        let exclude_types = effective_exclude_types(config);
        let filtered_api;
        let api = if exclude_types.is_empty() {
            api
        } else {
            filtered_api = api_without_excluded_types(api, &exclude_types);
            &filtered_api
        };
        let layout = ProjectLayout::resolve(config);
        let mut files = emit_android_project_scaffolding(config, &layout);
        files.extend(gen_jni_skeleton::emit(config, &layout.package_root));
        let deduped_api = effective_codegen_api(api, config);
        files.extend(gen_bindings::emit(&deduped_api, config, &layout.kotlin_source_dir));
        // Built from `deduped_api` — the exact surface the Kotlin emitter above just wrote —
        // so the seed can never assert against a declaration that was filtered out. ~keep
        files.push(gen_seed_test::emit(&deduped_api, config, &layout.package_root));

        apply_kotlin_post_processing(&mut files);
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

    /// Kotlin/Android registration lives on the generated `object <Trait>Bridge`, whose method
    /// names are fixed rather than taken from `register_fn`/`unregister_fn`/`clear_fn` — those
    /// only decide *whether* the `unregister`/`clearAll` methods exist. `register` is always
    /// emitted. Mirrors the gates in `gen_bindings::trait_interfaces::emit_trait_interfaces`,
    /// which is the only caller of the bridge-object emitter. ~keep
    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        let excluded_functions = naming::excluded_function_names(config);
        config
            .trait_bridges_for(Language::KotlinAndroid)
            .filter(|bridge| {
                bridge
                    .param_name
                    .as_deref()
                    .is_none_or(|param| !excluded_functions.contains(param))
            })
            .filter_map(|bridge| {
                let trait_def = api
                    .types
                    .iter()
                    .find(|typ| typ.is_trait && typ.name == bridge.trait_name && !typ.binding_excluded)?;
                let bridge_object = naming::bridge_object_name(&trait_def.name);
                Some(TraitBridgeRegistrationSurface {
                    trait_name: trait_def.name.clone(),
                    register_symbol: Some(format!("{bridge_object}.{}", trait_bridge::REGISTER_METHOD)),
                    unregister_symbol: bridge
                        .unregister_fn
                        .as_ref()
                        .map(|_| format!("{bridge_object}.{}", trait_bridge::UNREGISTER_METHOD)),
                    clear_symbol: bridge
                        .clear_fn
                        .as_ref()
                        .map(|_| format!("{bridge_object}.{}", trait_bridge::CLEAR_METHOD)),
                })
            })
            .collect()
    }
}

fn emit_android_project_scaffolding(config: &ResolvedCrateConfig, layout: &ProjectLayout) -> Vec<GeneratedFile> {
    let root = &layout.package_root;
    vec![
        android_project_file(root, "build.gradle.kts", gen_build_gradle::emit(config)),
        android_project_file(root, "gradle.properties", gen_gradle_properties::emit()),
        android_project_file(root, "settings.gradle.kts", gen_settings_gradle::emit(config)),
        android_project_file(
            root,
            "gradle/wrapper/gradle-wrapper.properties",
            gradle_wrapper::render_gradle_wrapper_properties(),
        ),
        android_project_file(root, "gradlew", gradle_wrapper::GRADLE_WRAPPER_UNIX.to_string()),
        android_project_file(root, "gradlew.bat", gradle_wrapper::GRADLE_WRAPPER_WINDOWS.to_string()),
        android_project_file(
            root,
            "gradle/wrapper/gradle-wrapper.jar",
            gradle_wrapper::get_gradle_wrapper_jar_base64(),
        ),
        android_project_file(root, "src/main/AndroidManifest.xml", gen_manifest::emit(config)),
        android_project_file(root, "consumer-rules.pro", gen_proguard::emit_consumer(config)),
        android_project_file(root, "proguard-rules.pro", gen_proguard::emit_module()),
        android_project_file(root, ".gitignore", gen_gitignore::emit()),
        android_project_file(root, ".editorconfig", gen_editorconfig::emit()),
    ]
}

fn android_project_file(root: &Path, relative_path: &str, content: String) -> GeneratedFile {
    GeneratedFile {
        path: root.join(relative_path),
        content,
        generated_header: false,
    }
}

/// The cfg-visible API surface for this backend, filtered by the features the Android artifact
/// is actually built with.
///
/// The configured list is expanded, and unioned with the core crate's own declared defaults where
/// active, through [`crate::codegen::cfg::enabled_features_for_language`] before it is used as the
/// enabled-feature set. `with_cfg_filtered_deep` matches a `#[cfg(feature = "X")]` gate against that set
/// LITERALLY, so a consumer who names a core-crate aggregate (the `android-target` shape) under
/// `[crates.kotlin_android].features` would otherwise have every `#[cfg(feature = "<member>")]`
/// item dropped from the Android surface even though cargo compiles them — silent underexposure
/// with no diagnostic, and disagreement with the JNI shim gate, which already expands. ~keep
fn effective_codegen_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
    let expanded = crate::codegen::cfg::enabled_features_for_language(config, Language::KotlinAndroid);
    let enabled_features: std::collections::HashSet<&str> = expanded.iter().map(String::as_str).collect();
    let mut api = api.with_cfg_filtered_deep(&enabled_features).with_deduped_functions();
    if let Some(android) = &config.kotlin_android {
        api.types
            .retain(|type_def| !android.capsule_types.contains_key(&type_def.name));
    }
    api
}

fn validate_capsule_ffi_parity(config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let Some(android) = &config.kotlin_android else {
        return Ok(());
    };
    crate::core::config::languages::require_shared_native_runtime(
        &android.capsule_types,
        android.shares_native_runtime,
        "kotlin_android",
    )?;
    let ffi_capsules = config.ffi.as_ref().map(|ffi| &ffi.capsule_types);
    let mut missing_ffi: Vec<_> = android
        .capsule_types
        .keys()
        .filter(|name| ffi_capsules.is_none_or(|capsules| !capsules.contains_key(*name)))
        .cloned()
        .collect();
    missing_ffi.sort_unstable();
    if !missing_ffi.is_empty() {
        anyhow::bail!(
            "kotlin_android capsule types require matching FFI capsule definitions: {}; \
             add each type under `[crates.ffi.capsule_types.<Type>]`",
            missing_ffi.join(", ")
        );
    }
    Ok(())
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

/// The Gradle project root this backend writes `build.gradle.kts` into — the directory a
/// `cd <dir> && gradle …` build, lint, test, or clean command must target.
///
/// [`crate::core::config::ResolvedCrateConfig::package_dir`] calls this instead of deriving the
/// directory a second time. `[crates.output].kotlin_android` may name either the Gradle project
/// root or the Kotlin source destination inside it, and only [`ProjectLayout`] knows how to tell
/// those two shapes apart; a second derivation that guessed would send the build command
/// somewhere this backend never wrote. ~keep
#[must_use]
pub fn project_root(config: &ResolvedCrateConfig) -> PathBuf {
    ProjectLayout::resolve(config).package_root
}

/// The Kotlin source destination this backend writes `<Module>.kt` and client sources into --
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/` by default, or the configured
/// `[crates.output].kotlin_android` path directly when it already names that source-set
/// directory.
///
/// [`crate::backends::kotlin::gen_bindings::jni_emitter::paths::jni_output_path`] calls this
/// instead of joining a filename directly onto `output_for("kotlin_android")`: that path may
/// name the Gradle project root rather than a source directory, and treating the project root
/// as a source directory wrote JNI-mode `.kt` files straight into it instead of under
/// `src/main/kotlin/<pkg>/`. Only [`ProjectLayout`] knows how to tell the two shapes apart. ~keep
#[must_use]
pub fn kotlin_source_dir(config: &ResolvedCrateConfig) -> PathBuf {
    ProjectLayout::resolve(config).kotlin_source_dir
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

/// Apply post-processing fixes to generated Kotlin files using the shared normalizer.
/// Fixes integer-like float literals that lack decimal points (e.g., "32" -> "32.0").
///
/// Uses `Path::extension` rather than `Path::ends_with`: the latter performs
/// component-wise matching, so `ends_with(".kt")` is always `false` for a file
/// named `Foo.kt` (the final component is `Foo.kt`, not `.kt`).
fn apply_kotlin_post_processing(files: &mut [GeneratedFile]) {
    for file in files {
        if file.path.extension().is_some_and(|ext| ext == "kt") {
            file.content = literal_normalizer::fix_float_literals(&file.content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_undeclared_capsule(shares_native_runtime: bool) -> ResolvedCrateConfig {
        let mut capsule_types = std::collections::HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            crate::core::config::HostCapsuleTypeConfig {
                host_type: "io.github.treesitter.ktreesitter.Language".to_string(),
                ..Default::default()
            },
        );
        ResolvedCrateConfig {
            kotlin_android: Some(crate::core::config::KotlinAndroidConfig {
                capsule_types,
                shares_native_runtime,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn undeclared_capsule_contract_is_rejected_by_the_gate() {
        let error = validate_capsule_ffi_parity(&config_with_undeclared_capsule(false))
            .expect_err("an undeclared capsule contract must not pass the gate");
        assert!(
            error.to_string().contains("cannot safely wrap native pointers"),
            "expected the capsule gate error, got: {error}"
        );
    }

    #[test]
    fn shares_native_runtime_is_read_from_config_and_clears_the_gate() {
        let error = validate_capsule_ffi_parity(&config_with_undeclared_capsule(true))
            .expect_err("the FFI-parity check still applies once the gate is cleared");
        assert!(
            !error.to_string().contains("cannot safely wrap native pointers"),
            "`shares_native_runtime = true` was ignored — the gate still fired: {error}"
        );
        assert!(
            error.to_string().contains("matching FFI capsule definitions"),
            "expected to reach the FFI-parity check, got: {error}"
        );
    }

    #[test]
    fn strip_kotlin_source_suffix_extracts_project_root() {
        let configured = Path::new("packages/kotlin-android/src/main/kotlin/dev/sample_crate/sample_crawler/android");
        let root = strip_kotlin_source_suffix(configured, "dev/sample_crate/sample_crawler/android");
        assert_eq!(root, Some(PathBuf::from("packages/kotlin-android")));
    }

    #[test]
    fn strip_kotlin_source_suffix_returns_none_when_suffix_missing() {
        let configured = Path::new("packages/kotlin-android");
        assert_eq!(strip_kotlin_source_suffix(configured, "dev/sample_crate"), None);
    }

    #[test]
    fn from_configured_derives_package_root_when_path_targets_kotlin_source() {
        let configured = Path::new("packages/kotlin-android/src/main/kotlin/dev/sample_crate/sample_crawler/android");
        let layout = ProjectLayout::from_configured(configured, "dev/sample_crate/sample_crawler/android");
        assert_eq!(layout.package_root, PathBuf::from("packages/kotlin-android"));
        assert_eq!(layout.kotlin_source_dir, PathBuf::from(configured));
    }

    #[test]
    fn from_configured_falls_back_to_legacy_when_path_is_project_root() {
        let configured = Path::new("packages/kotlin-android");
        let layout = ProjectLayout::from_configured(configured, "dev/sample_crate");
        assert_eq!(layout.package_root, PathBuf::from("packages/kotlin-android"));
        assert_eq!(
            layout.kotlin_source_dir,
            PathBuf::from("packages/kotlin-android/src/main/kotlin/dev/sample_crate")
        );
    }

    #[test]
    fn apply_kotlin_post_processing_fixes_double_literals_in_named_kt_files() {
        let mut files = vec![GeneratedFile {
            path: PathBuf::from("src/main/kotlin/dev/sample_crate/OcrQualityThresholds.kt"),
            content: "    val minNonWhitespacePerPage: Double = 32,\n".to_string(),
            generated_header: true,
        }];
        apply_kotlin_post_processing(&mut files);
        assert_eq!(files[0].content, "    val minNonWhitespacePerPage: Double = 32.0,\n");
    }

    #[test]
    fn apply_kotlin_post_processing_skips_non_kotlin_files() {
        let mut files = vec![GeneratedFile {
            path: PathBuf::from("build.gradle.kts"),
            content: "ext = 32".to_string(),
            generated_header: false,
        }];
        apply_kotlin_post_processing(&mut files);
        assert_eq!(files[0].content, "ext = 32");
    }

    #[test]
    fn disabled_feature_functions_are_absent_from_android_facade() {
        use crate::core::config::NewAlefConfig;

        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = ["mobile"]
"#,
        )
        .expect("fixture config parses");
        let config = raw.resolve().expect("fixture config resolves").remove(0);
        let function = |name: &str, rust_path: &str, cfg: &str| crate::core::ir::FunctionDef {
            name: name.into(),
            rust_path: rust_path.into(),
            return_type: crate::core::ir::TypeRef::String,
            cfg: Some(cfg.into()),
            ..Default::default()
        };
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            functions: vec![
                function("decode_sample", "demo::decoder::decode_sample", "feature = \"decoder\""),
                function(
                    "decode_sample",
                    "demo::decode_sample",
                    "all(feature = \"mobile\", not(feature = \"decoder\"))",
                ),
                function(
                    "decoder_details",
                    "demo::decoder::decoder_details",
                    "feature = \"decoder\"",
                ),
            ],
            ..Default::default()
        };

        let effective_api = effective_codegen_api(&api, &config);
        let files = gen_bindings::emit(&effective_api, &config, Path::new("generated"));
        let kotlin = files
            .iter()
            .filter(|file| file.path.extension().is_some_and(|extension| extension == "kt"))
            .map(|file| file.content.as_str())
            .collect::<String>();

        assert!(
            kotlin.contains("decodeSample"),
            "available fallback must remain public: {kotlin}"
        );
        assert!(
            !kotlin.contains("decoderDetails"),
            "disabled feature API must be omitted: {kotlin}"
        );
    }

    /// A configured aggregate feature must expose every member gate it turns on.
    ///
    /// `[crates.kotlin_android].features = ["mobile-target"]` with a core crate declaring
    /// `mobile-target = ["decoder"]` compiles `#[cfg(feature = "decoder")]` items into the
    /// Android artifact, so they must appear in the Kotlin surface. Matching the configured list
    /// literally drops them with no diagnostic — API present on desktop, absent on Android.
    #[test]
    fn aggregate_feature_members_reach_the_android_surface() {
        use crate::core::config::NewAlefConfig;

        let dir = tempfile::tempdir().expect("tempdir");
        let core_dir = dir.path().join("crates").join("demo");
        std::fs::create_dir_all(core_dir.join("src")).expect("create core crate dir");
        std::fs::write(
            core_dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n\n[features]\ndefault = []\n\
             mobile-target = [\"decoder\"]\ndecoder = []\nunrelated = []\n",
        )
        .expect("write core Cargo.toml");

        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = ["mobile-target"]
"#,
        )
        .expect("fixture config parses");
        let mut config = raw.resolve().expect("fixture config resolves").remove(0);
        config.workspace_root = Some(dir.path().to_path_buf());

        let api = ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            functions: vec![
                crate::core::ir::FunctionDef {
                    name: "decoder_details".into(),
                    rust_path: "demo::decoder::decoder_details".into(),
                    return_type: crate::core::ir::TypeRef::String,
                    cfg: Some("feature = \"decoder\"".into()),
                    ..Default::default()
                },
                crate::core::ir::FunctionDef {
                    name: "unrelated_details".into(),
                    rust_path: "demo::unrelated::unrelated_details".into(),
                    return_type: crate::core::ir::TypeRef::String,
                    cfg: Some("feature = \"unrelated\"".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let effective_api = effective_codegen_api(&api, &config);
        let files = gen_bindings::emit(&effective_api, &config, Path::new("generated"));
        let kotlin = files
            .iter()
            .filter(|file| file.path.extension().is_some_and(|extension| extension == "kt"))
            .map(|file| file.content.as_str())
            .collect::<String>();

        assert!(
            kotlin.contains("decoderDetails"),
            "`mobile-target` enables `decoder` in the core manifest, so the gated item must be \
             present in the Android surface:\n{kotlin}"
        );
        assert!(
            !kotlin.contains("unrelatedDetails"),
            "a feature the aggregate does not reach must stay filtered out:\n{kotlin}"
        );
    }

    /// `generate_bindings` used to abort the whole process — `gen_build_gradle::emit`
    /// panicked rather than returning an error — whenever `[crates.package_metadata]` /
    /// `[crates.scaffold]` omitted `repository` or `license`. Those fields only feed the
    /// published POM's optional URL/SCM/license sections, so a plain `alef generate` for a
    /// crate that had not configured Maven Central metadata yet should still succeed. ~keep
    #[test]
    fn generate_bindings_succeeds_without_package_metadata() {
        use crate::core::config::NewAlefConfig;

        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"

[crates.jni]
"#,
        )
        .expect("fixture config parses");
        let config = raw.resolve().expect("fixture config resolves").remove(0);
        assert!(
            config.package_metadata.is_none() && config.scaffold.is_none(),
            "fixture must not configure repository/license"
        );

        let api = ApiSurface::default();
        let files = KotlinAndroidBackend
            .generate_bindings(&api, &config)
            .expect("kotlin_android generation must not require repository/license metadata");

        let gradle = files
            .iter()
            .find(|file| file.path.ends_with("build.gradle.kts"))
            .expect("build.gradle.kts must still be emitted");
        assert!(
            !gradle.content.contains("scm {"),
            "an unconfigured repository must omit the scm block rather than invent one:\n{}",
            gradle.content
        );
        assert!(
            !gradle.content.contains("licenses {"),
            "an unconfigured license must omit the licenses block rather than invent one:\n{}",
            gradle.content
        );
    }
}
