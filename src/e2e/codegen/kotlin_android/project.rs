use crate::backends::kotlin_android::naming;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::path::PathBuf;

use super::enum_fixtures::is_enum_typed;
use super::gradle::{
    render_build_gradle_kotlin_android, render_gradle_properties, render_settings_gradle_kotlin_android,
};
use super::gradle_wrapper::{
    GRADLE_WRAPPER_UNIX, GRADLE_WRAPPER_WINDOWS, get_gradle_wrapper_jar_base64, render_gradle_wrapper_properties,
};
use crate::e2e::codegen::kotlin;

pub(super) fn generate(
    groups: &[FixtureGroup],
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
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
    let result_var = &call.result_var;

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
    files.push(GeneratedFile {
        path: output_base.join("build.gradle.kts"),
        content: render_build_gradle_kotlin_android(
            &kotlin_pkg_id,
            &maven_coordinate,
            e2e_config.dep_mode,
            &jni_lib_name,
            &jni_crate_path,
        ),
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

    if needs_mock_server {
        files.push(GeneratedFile {
            path: test_base.join("MockServerListener.kt"),
            content: kotlin::render_mock_server_listener_kt(&kotlin_pkg_id),
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
    for group in groups {
        let active: Vec<&Fixture> = group
            .fixtures
            .iter()
            .filter(|f| crate::e2e::codegen::should_include_fixture(f, lang, e2e_config))
            .filter(|f| f.visitor.is_none())
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
        );
        files.push(GeneratedFile {
            path: test_base.join(&class_file_name),
            content,
            generated_header: true,
        });
    }

    Ok(files)
}
