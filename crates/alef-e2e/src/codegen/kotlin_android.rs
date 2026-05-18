//! Kotlin Android e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates host-JVM tests that validate the AAR-bundled Java facade and Kotlin wrapper
//! via JNA against libkreuzberg_ffi. Tests are emitted to `e2e/kotlin_android/src/test/kotlin/`
//! without requiring an Android emulator — the tests run directly on the host JVM against
//! the shared library.

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::field_access::FieldResolver;
use crate::fixture::{Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::template_versions::{maven, toolchain};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::path::PathBuf;

use super::E2eCodegen;
use super::kotlin;

/// Kotlin Android e2e code generator.
/// Emits a host-JVM test project that depends on the AAR-bundled Java facade
/// and Kotlin wrapper via sourceSets and JNA, without requiring an Android emulator.
pub struct KotlinAndroidE2eCodegen;

impl E2eCodegen for KotlinAndroidE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[alef_core::ir::TypeDef],
        _enums: &[alef_core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
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
        files.push(GeneratedFile {
            path: output_base.join("build.gradle.kts"),
            content: render_build_gradle_kotlin_android(
                &pkg_name,
                &kotlin_pkg_id,
                &kotlin_android_version,
                e2e_config.dep_mode,
                needs_mock_server,
            ),
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
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &HashSet::new(),
        );

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

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_file_name = format!("{}Test.kt", sanitize_filename(&group.category).to_upper_camel_case());
            let content = kotlin::render_test_file_android(
                &group.category,
                &active,
                &class_name,
                &function_name,
                &kotlin_pkg_id,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
                e2e_config,
                &type_enum_fields,
            );
            files.push(GeneratedFile {
                path: test_base.join(&class_file_name),
                content,
                generated_header: true,
            });

            // Instrumented Android test for on-device emulator runs.
            // Lives in src/androidTest/ and uses @RunWith(AndroidJUnit4::class).
            let mut android_test_base = output_base.join("src").join("androidTest").join("kotlin");
            for segment in kotlin_pkg_id.split('.') {
                android_test_base = android_test_base.join(segment);
            }
            let android_test_base = android_test_base.join("e2e");
            files.push(GeneratedFile {
                path: android_test_base.join(class_file_name),
                content: render_android_instrumented_test(
                    &group.category,
                    &active,
                    &class_name,
                    &function_name,
                    &kotlin_pkg_id,
                    result_var,
                    &pkg_name,
                ),
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "kotlin_android"
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `ty` is a `Named(T)` reference (or `Optional<Named(T)>`)
/// where `T` is **not** a known struct name. Such fields are enum-typed and
/// must route through `.getValue()` in generated assertions.
fn is_enum_typed(ty: &alef_core::ir::TypeRef, struct_names: &HashSet<&str>) -> bool {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => !struct_names.contains(name.as_str()),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if !struct_names.contains(name.as_str()))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render build.gradle.kts for the kotlin_android e2e project.
///
/// This is an Android library project (applies `com.android.library`) so that
/// the `android { }` DSL — including Gradle Managed Devices — resolves at
/// Kotlin script compile time. The host-JVM test sources live in
/// `src/test/kotlin/` and run against the shared native library via JNA.
fn render_build_gradle_kotlin_android(
    _pkg_name: &str,
    kotlin_pkg_id: &str,
    _pkg_version: &str,
    _dep_mode: crate::config::DependencyMode,
    needs_mock_server: bool,
) -> String {
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let android_gradle_plugin = maven::ANDROID_GRADLE_PLUGIN;
    let junit = maven::JUNIT;
    let jackson = maven::JACKSON_E2E;
    // E2E tests run on the host JVM (not Android), so pick a target that
    // matches the JUnit Jupiter baseline (5.x → JVM 11, 6.x → JVM 17). The
    // Android library itself still ships at ANDROID_JVM_TARGET for runtime
    // compat; this only affects the host-side gradle test project.
    let jvm_target = if junit.starts_with("6.") {
        "17"
    } else {
        toolchain::ANDROID_JVM_TARGET
    };
    let jna = maven::JNA;
    let jspecify = maven::JSPECIFY;
    let coroutines = maven::KOTLINX_COROUTINES_CORE;
    let launcher_dep = if needs_mock_server {
        format!(r#"    testImplementation("org.junit.platform:junit-platform-launcher:{junit}")"#)
    } else {
        String::new()
    };

    format!(
        r#"import com.android.build.api.dsl.ManagedVirtualDevice
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {{
    id("com.android.library") version "{android_gradle_plugin}"
    kotlin("android") version "{kotlin_plugin}"
}}

group = "{kotlin_pkg_id}"
version = "0.1.0"

android {{
    namespace = "{kotlin_pkg_id}.e2e"
    compileSdk = 35

    defaultConfig {{
        minSdk = 21
    }}

    compileOptions {{
        sourceCompatibility = JavaVersion.VERSION_{jvm_target}
        targetCompatibility = JavaVersion.VERSION_{jvm_target}
    }}

    sourceSets {{
        getByName("test") {{
            // Include the AAR-bundled Java facade as test sources
            java.srcDir("../../packages/kotlin-android/src/main/java")
            // Include the AAR-bundled Kotlin wrapper as test sources
            kotlin.srcDir("../../packages/kotlin-android/src/main/kotlin")
        }}
    }}

    testOptions {{
        // Gradle Managed Virtual Devices for on-device instrumented tests.
        // Run: ./gradlew pixel6api34DebugAndroidTest
        managedDevices {{
            devices {{
                create<ManagedVirtualDevice>("pixel6api34") {{
                    device = "Pixel 6"
                    apiLevel = 34
                    systemImageSource = "aosp"
                }}
            }}
        }}
    }}
}}

kotlin {{
    compilerOptions {{
        jvmTarget = JvmTarget.JVM_{jvm_target}
    }}
}}

// Repositories declared in settings.gradle.kts via
// dependencyResolutionManagement (FAIL_ON_PROJECT_REPOS). Re-declaring them
// here triggers Gradle "repository was added by build file" errors.

dependencies {{
    // JNA for loading the native library from java.library.path
    testImplementation("net.java.dev.jna:jna:{jna}")

    // Jackson for JSON assertion helpers
    testImplementation("com.fasterxml.jackson.core:jackson-annotations:{jackson}")
    testImplementation("com.fasterxml.jackson.core:jackson-databind:{jackson}")
    testImplementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")

    // jackson-module-kotlin registers constructors/properties for Kotlin data
    // classes, which have no default constructor and cannot be deserialized by
    // plain Jackson without this module.
    testImplementation("com.fasterxml.jackson.module:jackson-module-kotlin:{jackson}")

    // jspecify for null-safety annotations on wrapped types
    testImplementation("org.jspecify:jspecify:{jspecify}")

    // Kotlin coroutines for async test helpers
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:{coroutines}")

    // JUnit 5 API and engine
    testImplementation("org.junit.jupiter:junit-jupiter-api:{junit}")
    testImplementation("org.junit.jupiter:junit-jupiter-engine:{junit}")
{launcher_dep}

    // Kotlin stdlib test helpers
    testImplementation(kotlin("test"))
}}

tasks.withType<Test> {{
    useJUnitPlatform()

    // Resolve the native library location (e.g., ../../target/release)
    val libPath = System.getProperty("kb.lib.path") ?: "${{rootDir}}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)

    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/
    workingDir = file("${{rootDir}}/../../test_documents")
}}
"#
    )
}

/// Render `settings.gradle.kts` for the kotlin_android e2e project.
///
/// Declares the plugin and dependency repositories Gradle needs to resolve
/// `com.android.library` (and Kotlin/Android transitive deps). Mirrors the
/// AAR-side settings emitter at `alef-backend-kotlin-android::gen_settings_gradle`.
fn render_settings_gradle_kotlin_android(pkg_name: &str) -> String {
    format!(
        r#"// Generated by alef. Do not edit by hand.

pluginManagement {{
    repositories {{
        google()
        mavenCentral()
        gradlePluginPortal()
    }}
}}

dependencyResolutionManagement {{
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {{
        google()
        mavenCentral()
    }}
}}

rootProject.name = "{pkg_name}-e2e"
"#
    )
}

/// Render an Android instrumented test class for a fixture group.
///
/// The generated class uses `@RunWith(AndroidJUnit4::class)` and loads the
/// native library via `System.loadLibrary` so tests can run on-device via the
/// Android emulator.
fn render_android_instrumented_test(
    category: &str,
    fixtures: &[&crate::fixture::Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    lib_name: &str,
) -> String {
    let test_class = format!("{}Test", category.to_upper_camel_case());
    let lib_snake = lib_name.replace('-', "_");
    let mut out = String::new();
    out.push_str(&format!("package {kotlin_pkg_id}.e2e\n\n"));
    out.push_str("import androidx.test.ext.junit.runners.AndroidJUnit4\n");
    out.push_str("import org.junit.BeforeClass\n");
    out.push_str("import org.junit.Test\n");
    out.push_str("import org.junit.runner.RunWith\n\n");
    out.push_str("@RunWith(AndroidJUnit4::class)\n");
    out.push_str(&format!("class {test_class} {{\n\n"));
    out.push_str("    companion object {\n");
    out.push_str("        @BeforeClass\n");
    out.push_str("        @JvmStatic\n");
    out.push_str("        fun loadNativeLibrary() {\n");
    out.push_str(&format!("            System.loadLibrary(\"{lib_snake}_jni\")\n"));
    out.push_str("        }\n");
    out.push_str("    }\n\n");
    for fixture in fixtures {
        let test_name = fixture.id.replace(['-', '.', ' '], "_");
        out.push_str("    @Test\n");
        out.push_str(&format!("    fun test_{test_name}() {{\n"));
        out.push_str(&format!("        val client = {class_name}()\n"));
        out.push_str(&format!(
            "        val {result_var} = client.{function_name}(/* fixture: {} */)\n",
            fixture.id
        ));
        out.push_str(&format!("        // TODO: assert {result_var} is not an error\n"));
        out.push_str("    }\n\n");
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the kotlin-android build.gradle.kts must declare
    /// `jackson-module-kotlin` so that Jackson can deserialize Kotlin data
    /// classes (which have no default constructor).  Without it, any test that
    /// calls `MAPPER.readValue(...)` against a Kotlin data class throws
    /// `InvalidDefinitionException: No suitable constructor found`.
    #[test]
    fn build_gradle_kotlin_android_includes_jackson_module_kotlin() {
        let output = render_build_gradle_kotlin_android(
            "liter-llm",
            "dev.kreuzberg.literllm.android",
            "1.0.0",
            crate::config::DependencyMode::Local,
            false,
        );
        assert!(
            output.contains("jackson-module-kotlin"),
            "build.gradle.kts must depend on jackson-module-kotlin, got:\n{output}"
        );
    }

    /// Regression: the e2e settings.gradle.kts must declare the
    /// `pluginManagement` block with `google()` and `gradlePluginPortal()` so
    /// Gradle can resolve `com.android.library`. Missing settings.gradle.kts
    /// causes `Plugin [id: 'com.android.library'] was not found` at config time.
    #[test]
    fn settings_gradle_kotlin_android_declares_plugin_repositories() {
        let output = render_settings_gradle_kotlin_android("liter-llm");
        assert!(
            output.contains("pluginManagement"),
            "settings.gradle.kts must declare pluginManagement block, got:\n{output}"
        );
        assert!(
            output.contains("google()"),
            "pluginManagement repositories must include google(), got:\n{output}"
        );
        assert!(
            output.contains("gradlePluginPortal()"),
            "pluginManagement repositories must include gradlePluginPortal(), got:\n{output}"
        );
        assert!(
            output.contains("rootProject.name = \"liter-llm-e2e\""),
            "rootProject.name must be derived from pkg_name, got:\n{output}"
        );
    }
}
