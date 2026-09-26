use crate::backends::kotlin_android::naming;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::cfg_feature_satisfied;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup, VISITOR_EXCLUDE_FUNCTION_NAME};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::path::PathBuf;

use super::enum_fixtures::is_enum_typed;
use super::gradle::{
    KotlinAndroidBuildGradleInputs, render_build_gradle_kotlin_android, render_gradle_properties,
    render_settings_gradle_kotlin_android,
};
use super::gradle_wrapper::{
    GRADLE_WRAPPER_UNIX, GRADLE_WRAPPER_WINDOWS, get_gradle_wrapper_jar_base64, render_gradle_wrapper_properties,
};
use crate::e2e::codegen::kotlin;

/// One test that `ExcludedBindingsTest.kt` renders as `@Disabled` rather than as a real
/// call, paired with the reason a human reads in the JUnit report.
#[derive(serde::Serialize)]
struct ExcludedFixtureEntry {
    name: String,
    reason: String,
}

/// True when `fixture`'s resolved call names a free function the core IR declares with a
/// `#[cfg(feature = "...")]` gate that `enabled_features` does not satisfy.
///
/// `[crates.kotlin_android].features` (see [`ResolvedCrateConfig::features_for_language`])
/// is the same feature list `backends::kotlin_android::effective_codegen_api` filters the
/// *binding* surface with via `ApiSurface::with_cfg_filtered_deep`. Before this check, the
/// e2e generator had no idea that filter existed: it emitted a call for every fixture whose
/// declared function name resolved in the *unfiltered* IR, so a fixture routed to
/// `manifest_languages` (gated on a `download` feature that
/// `[crates.kotlin_android].features = ["serde"]` does not enable) produced a Kotlin test
/// calling `SampleFacade.manifestLanguages()` — a symbol the binding generator
/// correctly never emitted. Free functions only: the gated family this was found
/// against has none of its members as methods, and a method's cfg would need agreement
/// across every same-named method the way `CallIr::signature` requires for its own checks,
/// which is out of scope for this fix.
fn function_cfg_gated_out(
    fixture: &Fixture,
    lang: &str,
    e2e_config: &E2eConfig,
    functions: &[crate::core::ir::FunctionDef],
    enabled_features: &HashSet<&str>,
) -> Option<String> {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let lookup_name = call_config.core_lookup_name(lang)?;
    let function = functions.iter().find(|f| f.name == lookup_name)?;
    let cfg = function.cfg.as_deref()?;
    if cfg_feature_satisfied(Some(cfg), enabled_features) {
        None
    } else {
        Some(cfg.to_string())
    }
}

pub(super) fn generate(
    groups: &[FixtureGroup],
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<Vec<GeneratedFile>> {
    let lang = "kotlin_android";
    let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

    let mut files = Vec::new();

    // Resolve call config with overrides.
    let call = &e2e_config.call;
    let overrides = call.overrides.get(lang);
    let _module_path = overrides
        .and_then(|o| o.module.as_ref())
        .cloned()
        .unwrap_or_else(|| call.module.clone());
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.clone());
    let class_name = overrides
        .and_then(|o| o.class.as_ref())
        .cloned()
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
    let result_var = call.effective_result_var();

    // Resolve package config.
    let kotlin_android_pkg = e2e_config.resolve_package("kotlin_android");
    let pkg_name = kotlin_android_pkg
        .as_ref()
        .and_then(|p| p.name.as_ref())
        .cloned()
        .unwrap_or_else(|| config.name.clone());

    // Resolve Kotlin package for generated tests.
    let _kotlin_android_pkg_path = kotlin_android_pkg
        .as_ref()
        .and_then(|p| p.path.as_deref())
        .unwrap_or("../../packages/kotlin-android");
    let kotlin_android_version = kotlin_android_pkg
        .as_ref()
        .and_then(|p| p.version.as_ref())
        .cloned()
        .or_else(|| config.resolved_version())
        .unwrap_or_else(|| "0.1.0".to_string());

    // Construct the Maven coordinate for the published Android AAR.
    // Format: `group_id:artifact_id:version` (e.g., `dev.sample_core:sample_core-android:5.0.0-rc.1`)
    let maven_group_id = naming::aar_group_id(config);
    let maven_artifact_id = naming::aar_artifact_id(config);
    let maven_coordinate = format!("{}:{}:{}", maven_group_id, maven_artifact_id, kotlin_android_version);

    // Use the kotlin_android crate's `package` config — not the generic
    // `config.kotlin_package()` accessor — so the generated tests live in
    // the same JVM package as the AAR's emitted types and can reference
    // them by simple name. `kotlin_package()` falls back to a
    // `com.github.<org>` derivation from the GitHub URL when
    // `[crates.kotlin] package` is absent, which produces a package
    // mismatch for AAR consumers that only configure
    // `[crates.kotlin_android] package`.
    //
    // Precedence: `[crates.e2e.packages.kotlin_android].module` (explicit
    // override) > `[crates.kotlin_android].package` > derived fallback
    // via `config.kotlin_package()`.
    let kotlin_pkg_id = kotlin_android_pkg
        .as_ref()
        .and_then(|p| p.module.clone())
        .or_else(|| config.kotlin_android.as_ref().and_then(|c| c.package.clone()))
        .unwrap_or_else(|| config.kotlin_package());

    // Detect whether any fixture needs the mock-server (HTTP fixtures or
    // fixtures with a mock_response/mock_responses). When present, emit a
    // JUnit Platform LauncherSessionListener that spawns the mock-server
    // before any test runs and a META-INF/services SPI manifest registering
    // it. Mirrors the Kotlin/JVM e2e pattern exactly.
    let needs_mock_server = groups
        .iter()
        .flat_map(|g| g.fixtures.iter())
        .any(|f| f.needs_mock_server());

    // Generate build.gradle.kts for the host JVM project.
    let jni_lib_name = config.jni_lib_name();
    let jni_crate_path = config.jni_crate_path();
    let capsule_types = config
        .kotlin_android
        .as_ref()
        .map(|android| android.capsule_types.clone())
        .unwrap_or_default();
    files.push(GeneratedFile {
        path: output_base.join("build.gradle.kts"),
        content: render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: &kotlin_pkg_id,
            maven_coordinate: &maven_coordinate,
            dep_mode: e2e_config.dep_mode,
            jni_lib_name: &jni_lib_name,
            jni_crate_path: &jni_crate_path,
            e2e_env: &e2e_config.env,
            capsule_types: &capsule_types,
            test_documents_path: &e2e_config.test_documents_relative_from(0),
            timeout_seconds: e2e_config.timeout_seconds,
        }),
        generated_header: false,
    });

    // Generate gradle.properties to configure Gradle toolchain auto-detection.
    // This allows the build to proceed on hosts without the specific JDK version.
    files.push(GeneratedFile {
        path: output_base.join("gradle.properties"),
        content: render_gradle_properties(),
        generated_header: false,
    });

    // Generate settings.gradle.kts so Gradle can resolve the AGP
    // (`com.android.library`) plugin from google()/gradlePluginPortal().
    // Without this file the e2e project fails at configuration time with
    // `Plugin [id: 'com.android.library'] was not found in any of the
    // following sources`.
    files.push(GeneratedFile {
        path: output_base.join("settings.gradle.kts"),
        content: render_settings_gradle_kotlin_android(&pkg_name),
        generated_header: false,
    });

    // In registry mode, generate gradle wrapper files so the test_app is self-contained
    // and doesn't require a system Gradle installation.
    if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
        files.push(GeneratedFile {
            path: output_base.join("gradle/wrapper/gradle-wrapper.properties"),
            content: render_gradle_wrapper_properties(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base.join("gradlew"),
            content: GRADLE_WRAPPER_UNIX.to_string(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base.join("gradlew.bat"),
            content: GRADLE_WRAPPER_WINDOWS.to_string(),
            generated_header: false,
        });
        // Emit gradle-wrapper.jar as base64-encoded content.
        // The file writer will detect the .jar extension and decode it automatically.
        files.push(GeneratedFile {
            path: output_base.join("gradle/wrapper/gradle-wrapper.jar"),
            content: get_gradle_wrapper_jar_base64(),
            generated_header: false,
        });
    }

    // Generate test files per category. Path mirrors the configured Kotlin
    // package so the package declaration in each test file matches its
    // filesystem location.
    let mut test_base = output_base.join("src").join("test").join("kotlin");
    for segment in kotlin_pkg_id.split('.') {
        test_base = test_base.join(segment);
    }
    let test_base = test_base.join("e2e");

    let visitor_is_excluded = config.kotlin_android.as_ref().is_some_and(|android| {
        android
            .exclude_functions
            .iter()
            .any(|name| name == VISITOR_EXCLUDE_FUNCTION_NAME)
    });
    let enabled_features: HashSet<&str> = config
        .features_for_language(Language::KotlinAndroid)
        .iter()
        .map(String::as_str)
        .collect();
    let mut excluded_entries: Vec<ExcludedFixtureEntry> = Vec::new();
    for fixture in groups.iter().flat_map(|group| group.fixtures.iter()) {
        if fixture.visitor.is_some() && visitor_is_excluded {
            excluded_entries.push(ExcludedFixtureEntry {
                name: sanitize_filename(&fixture.id),
                reason: "visitor is excluded by crates.kotlin_android.exclude_functions".to_string(),
            });
            continue;
        }
        if let Some(cfg) = function_cfg_gated_out(fixture, lang, e2e_config, functions, &enabled_features) {
            // The cfg string comes from `#[cfg(...)]` and may itself contain double quotes
            // (`feature = "download"`); this reason is spliced into a Kotlin string literal
            // (`@Disabled("...")`) with no escaping downstream, so quotes are stripped here
            // rather than passed through raw.
            let cfg_display = cfg.replace('"', "");
            excluded_entries.push(ExcludedFixtureEntry {
                name: sanitize_filename(&fixture.id),
                reason: format!("call is gated on {cfg_display}, which crates.kotlin_android.features does not enable"),
            });
        }
    }
    if !excluded_entries.is_empty() {
        files.push(GeneratedFile {
            path: test_base.join("ExcludedBindingsTest.kt"),
            content: crate::e2e::template_env::render(
                "kotlin_android/excluded_fixtures.kt.jinja",
                minijinja::context! {
                    package_name => kotlin_pkg_id.clone(),
                    entries => excluded_entries,
                },
            ),
            generated_header: true,
        });
    }

    if needs_mock_server {
        files.push(GeneratedFile {
            path: test_base.join("MockServerListener.kt"),
            content: kotlin::render_mock_server_listener_kt(&kotlin_pkg_id, &e2e_config.alt_host),
            generated_header: true,
        });
        files.push(GeneratedFile {
            path: output_base
                .join("src")
                .join("test")
                .join("resources")
                .join("META-INF")
                .join("services")
                .join("org.junit.platform.launcher.LauncherSessionListener"),
            content: format!("{kotlin_pkg_id}.e2e.MockServerListener\n"),
            generated_header: false,
        });
    }

    // Resolve options_type from override.
    let options_type = overrides.and_then(|o| o.options_type.clone());

    // Build a map from TypeDef name → set of field names whose Rust type
    // is a `Named(T)` reference where `T` is NOT itself a known struct.
    // Those fields are enum-typed and should route through `.getValue()` in
    // generated assertions automatically, even without an explicit per-call
    // `enum_fields` override in the alef.toml.
    let struct_names: HashSet<&str> = type_defs.iter().map(|td| td.name.as_str()).collect();
    let type_enum_fields: std::collections::HashMap<String, HashSet<String>> = type_defs
        .iter()
        .filter_map(|td| {
            let enum_field_names: HashSet<String> = td
                .fields
                .iter()
                .filter(|field| is_enum_typed(&field.ty, &struct_names))
                .map(|field| field.name.clone())
                .collect();
            if enum_field_names.is_empty() {
                None
            } else {
                Some((td.name.clone(), enum_field_names))
            }
        })
        .collect();

    // kotlin_android lacks a JNI trait-handle bridge (see alef-backend-jni follow-up), so
    // [crates.kotlin_android] excludes the visitor function. Fixtures whose payload uses
    // a visitor cannot be exercised through this binding — skip any visitor-using fixture.
    // Also skip any fixture whose call resolves to a core function [crates.kotlin_android]
    // does not compile in under its configured `features` — see `function_cfg_gated_out`.
    for group in groups {
        let active: Vec<&Fixture> = group
            .fixtures
            .iter()
            .filter(|f| crate::e2e::codegen::should_include_fixture(f, lang, e2e_config))
            .filter(|fixture| !(fixture.visitor.is_some() && visitor_is_excluded))
            .filter(|fixture| function_cfg_gated_out(fixture, lang, e2e_config, functions, &enabled_features).is_none())
            .collect();

        if active.is_empty() {
            continue;
        }

        let class_file_name = format!("{}Test.kt", sanitize_filename(&group.category).to_upper_camel_case());

        // Emit JUnit host-JVM tests under src/test/kotlin/.
        // Tests run via `gradle test` on the host JVM without requiring an Android device/emulator.
        let content = kotlin::render_test_file_android(
            &group.category,
            &active,
            &class_name,
            &function_name,
            &kotlin_pkg_id,
            result_var,
            &e2e_config.call.args,
            options_type.as_deref(),
            result_is_simple,
            e2e_config,
            &type_enum_fields,
            config,
            type_defs,
            enums,
            functions,
        )?;
        files.push(GeneratedFile {
            path: test_base.join(&class_file_name),
            content,
            generated_header: true,
        });
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{FunctionDef, TypeRef};

    /// A resolved config for a crate whose `[crates.kotlin_android].features` does not
    /// include `download` — the shape the tree-sitter-language-pack regression was found
    /// against (`features = ["serde"]`).
    fn config_excluding_download_feature() -> ResolvedCrateConfig {
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
features = ["serde"]
"#,
        )
        .expect("fixture config parses");
        raw.resolve().expect("fixture config resolves").remove(0)
    }

    /// The defect this pins: `manifest_languages` is gated on `#[cfg(feature = "download")]`
    /// in the Rust core. `backends::kotlin_android::effective_codegen_api` correctly drops it
    /// from the compiled facade because `[crates.kotlin_android].features` does not enable
    /// `download` — but before `function_cfg_gated_out` existed, this e2e generator had no
    /// way to know that and emitted `TreeSitterLanguagePack.manifestLanguages()` into a real
    /// test anyway, producing a Kotlin "Unresolved reference" compile failure. The fixture
    /// must now render as a `@Disabled` entry in `ExcludedBindingsTest.kt` instead, the same
    /// way a visitor-excluded fixture already does.
    #[test]
    fn cfg_gated_out_function_is_excluded_instead_of_dangling() {
        let config = config_excluding_download_feature();
        let functions = [FunctionDef {
            name: "manifest_languages".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"download\"".into()),
            ..FunctionDef::default()
        }];
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "manifest_languages".into();
        e2e_config.call.result_var = "result".into();
        let fixture = Fixture {
            id: "manifest_languages_smoke".into(),
            description: "List manifest languages".into(),
            category: Some("download".into()),
            ..Fixture::default()
        };
        let groups = [FixtureGroup {
            category: "download".into(),
            fixtures: vec![fixture],
        }];

        let files = generate(&groups, &e2e_config, &config, &[], &[], &functions).expect("generation succeeds");

        for file in &files {
            assert!(
                !file.content.contains("manifestLanguages("),
                "{} must not call manifestLanguages(): the binding never declares it under \
                 `features = [\"serde\"]`\n{}",
                file.path.display(),
                file.content
            );
        }
        let excluded = files
            .iter()
            .find(|f| f.path.ends_with("ExcludedBindingsTest.kt"))
            .unwrap_or_else(|| panic!("expected an ExcludedBindingsTest.kt naming the gated-out fixture"));
        assert!(
            excluded.content.contains("manifest_languages_smoke"),
            "{}",
            excluded.content
        );
        assert!(
            excluded.content.contains("feature = download"),
            "the skip reason should name the unsatisfied cfg gate:\n{}",
            excluded.content
        );
    }

    /// The companion positive control: once the language enables the gating feature, the
    /// same fixture must render as a normal call again, not stay excluded forever.
    #[test]
    fn cfg_satisfied_function_is_not_excluded() {
        let mut config = config_excluding_download_feature();
        config.kotlin_android = config.kotlin_android.map(|mut android| {
            android.features = Some(vec!["serde".to_string(), "download".to_string()]);
            android
        });
        let functions = [FunctionDef {
            name: "manifest_languages".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"download\"".into()),
            ..FunctionDef::default()
        }];
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "manifest_languages".into();
        e2e_config.call.result_var = "result".into();
        let fixture = Fixture {
            id: "manifest_languages_smoke".into(),
            description: "List manifest languages".into(),
            category: Some("download".into()),
            ..Fixture::default()
        };
        let groups = [FixtureGroup {
            category: "download".into(),
            fixtures: vec![fixture],
        }];

        let files = generate(&groups, &e2e_config, &config, &[], &[], &functions).expect("generation succeeds");

        assert!(
            files.iter().any(|f| f.content.contains("manifestLanguages(")),
            "expected a real call to manifestLanguages() once `download` is enabled"
        );
        assert!(
            !files.iter().any(|f| f.path.ends_with("ExcludedBindingsTest.kt")),
            "no fixture should be excluded once the gating feature is enabled"
        );
    }

    /// Regression: local-mode build.gradle.kts's `workingDir` assignment must be guarded
    /// on the test_documents directory's existence, the same way the plain-Kotlin
    /// generator (`kotlin::project::render_build_gradle`) already is. Gradle test workers
    /// fail to fork ("Gradle Test Executor N ... not in started or detached state", with
    /// the real fork `IOException` masked and no assertion text at all) when `workingDir`
    /// points at a directory that does not exist -- reproduced against a consumer whose
    /// `test_documents/` fixture directory has zero tracked files in a fresh checkout.
    #[test]
    fn build_gradle_local_mode_guards_working_dir_on_existence() {
        let config = ResolvedCrateConfig::default();
        let e2e_config = E2eConfig::default();

        let files = generate(&[], &e2e_config, &config, &[], &[], &[]).expect("generation succeeds");

        let build_gradle = files
            .iter()
            .find(|f| f.path.ends_with("build.gradle.kts"))
            .expect("build.gradle.kts must be generated");
        assert!(
            build_gradle.content.contains(".isDirectory"),
            "workingDir must be guarded on directory existence (mirrors \
             kotlin::project::render_build_gradle), got:\n{}",
            build_gradle.content
        );
        assert!(
            build_gradle.content.contains("workingDir = testDocuments"),
            "expected a guarded `workingDir = testDocuments` assignment, got:\n{}",
            build_gradle.content
        );
    }

    /// Regression: the test-documents directory name in the generated `workingDir` must
    /// come from `E2eConfig::test_documents_dir` (via `test_documents_relative_from`), not
    /// a hard-coded `"test_documents"` literal -- see CLAUDE.md's `project-agnostic-codegen`
    /// rule. A consumer that configures a non-default `test_documents_dir` must see that
    /// name reflected in the generated build.gradle.kts.
    #[test]
    fn build_gradle_local_mode_working_dir_uses_configured_test_documents_dir() {
        let config = ResolvedCrateConfig::default();
        let e2e_config = E2eConfig {
            test_documents_dir: "fixture_files".to_string(),
            ..E2eConfig::default()
        };

        let files = generate(&[], &e2e_config, &config, &[], &[], &[]).expect("generation succeeds");

        let build_gradle = files
            .iter()
            .find(|f| f.path.ends_with("build.gradle.kts"))
            .expect("build.gradle.kts must be generated");
        assert!(
            build_gradle.content.contains("../../fixture_files"),
            "workingDir must resolve the configured test_documents_dir, got:\n{}",
            build_gradle.content
        );
        assert!(
            !build_gradle.content.contains("../../test_documents"),
            "must not hard-code the literal `test_documents`, got:\n{}",
            build_gradle.content
        );
    }
}
