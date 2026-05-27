//! Kotlin Android e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates host-JVM tests that validate the AAR-bundled Java facade and Kotlin wrapper
//! via JNA against libsample_core_ffi. Tests are emitted to `e2e/kotlin_android/src/test/kotlin/`
//! without requiring an Android emulator — the tests run directly on the host JVM against
//! the shared library.

use crate::backends::kotlin_android::naming;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions::{maven, toolchain};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
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
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
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
        files.push(GeneratedFile {
            path: output_base.join("build.gradle.kts"),
            content: render_build_gradle_kotlin_android(
                &pkg_name,
                &kotlin_pkg_id,
                &kotlin_android_version,
                &maven_coordinate,
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

        // kotlin_android lacks a JNI trait-handle bridge (see alef-backend-jni TODO), so
        // [crates.kotlin_android] excludes the visitor function. Fixtures whose payload uses
        // a visitor cannot be exercised through this binding — skip any visitor-using fixture.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
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
fn is_enum_typed(ty: &crate::core::ir::TypeRef, struct_names: &HashSet<&str>) -> bool {
    use crate::core::ir::TypeRef;
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
/// In local mode: sources from `../../packages/kotlin-android/` are compiled
/// directly into the test project via `sourceSets`. In registry mode: the
/// published Maven artifact (from `maven_coordinate`) is declared as a
/// `testImplementation` dependency and `sourceSets` are not emitted.
///
/// This is an Android library project (applies `com.android.library`) so that
/// the `android { }` DSL — including Gradle Managed Devices — resolves at
/// Kotlin script compile time. The host-JVM test sources live in
/// `src/test/kotlin/` and run against the shared native library via JNA.
fn render_build_gradle_kotlin_android(
    _pkg_name: &str,
    kotlin_pkg_id: &str,
    _pkg_version: &str,
    maven_coordinate: &str,
    dep_mode: crate::e2e::config::DependencyMode,
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

    // In registry mode: depend on the published Maven artifact and declare
    // mavenCentral()/google() repos explicitly so the test_app is standalone.
    // No host-JVM tests are emitted (the AAR lacks JNI libraries for host machines).
    // In local mode: wire workspace sources directly via sourceSets so no
    // publish step is needed during development.
    let (source_sets_block, artifact_dep, tasks_block) = if dep_mode == crate::e2e::config::DependencyMode::Registry {
        let artifact = format!(
            r#"    // Published Android AAR from Maven Central (verifies artifact resolution)
    implementation("{maven_coordinate}")"#
        );
        // In registry mode no host-JVM tests run (see generate() for rationale).
        // A simple compile check verifies the AAR resolves correctly.
        let tasks = String::new();
        (String::new(), artifact, tasks)
    } else {
        let src_sets = r#"
    sourceSets {
        getByName("test") {
            // Include the AAR-bundled Java facade as test sources
            java.srcDir("../../packages/kotlin-android/src/main/java")
            // Include the AAR-bundled Kotlin wrapper as test sources
            kotlin.srcDir("../../packages/kotlin-android/src/main/kotlin")
        }
    }
"#;
        // In local mode wire JNA so the native library can be loaded from
        // the workspace target directory.
        let tasks = r#"tasks.withType<Test> {
    useJUnitPlatform()

    // Resolve the native library location (e.g., ../../target/release)
    val libPath = System.getProperty("kb.lib.path") ?: "${rootDir}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)

    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/
    workingDir = file("${rootDir}/../../test_documents")
}"#;
        (src_sets.to_string(), String::new(), tasks.to_string())
    };

    // Test dependencies are always needed for host-JVM tests (both Local and Registry modes).
    let test_deps = format!(
        r#"    // Jackson for JSON assertion helpers
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

    // JNA for loading the native library from java.library.path
    testImplementation("net.java.dev.jna:jna:{jna}")
"#
    );

    format!(
        r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

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
    }}{source_sets_block}
    testOptions {{
        // Host JVM unit tests: no Android device/emulator required.
        // Tests run against the published AAR and JVM-side deps via `gradle test`.
        unitTests {{
            isReturnDefaultValues = true
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
{artifact_dep}
{test_deps}
}}

{tasks_block}
"#
    )
}

/// Render `settings.gradle.kts` for the kotlin_android e2e project.
///
/// Declares the plugin and dependency repositories Gradle needs to resolve
/// `com.android.library` (and Kotlin/Android transitive deps). Mirrors the
/// AAR-side settings emitter at `alef-backend-kotlin-android::gen_settings_gradle`.
fn render_settings_gradle_kotlin_android(pkg_name: &str) -> String {
    let project_name = sanitize_gradle_project_name(pkg_name);
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

rootProject.name = "{project_name}-e2e"
"#
    )
}

/// Derive a Gradle-safe `rootProject.name` from a registry package coordinate.
///
/// Registry-mode `pkg_name` is often a Maven coordinate (`group:artifact`)
/// because it's used verbatim as a build-script dependency string. Gradle
/// rejects project names containing any of `[/, \, :, <, >, ", ?, *, |]`,
/// so we take the artifact segment after the last `:` and replace any
/// remaining reserved characters with `-`.
fn sanitize_gradle_project_name(pkg_name: &str) -> String {
    let artifact = pkg_name.rsplit(':').next().unwrap_or(pkg_name);
    artifact
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '"' | '?' | '*' | '|' => '-',
            other => other,
        })
        .collect()
}

/// Render an Android instrumented test class for a fixture group.
///
/// The generated class uses `@RunWith(AndroidJUnit4::class)` and loads the
/// native library via `System.loadLibrary` so tests can run on-device via the
/// Android emulator.
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
            "sample-llm",
            "dev.sample_crate.samplellm.android",
            "1.0.0",
            "dev.sample_crate:sample-llm-android:1.0.0",
            crate::e2e::config::DependencyMode::Local,
            false,
        );
        assert!(
            output.contains("jackson-module-kotlin"),
            "build.gradle.kts must depend on jackson-module-kotlin, got:\n{output}"
        );
    }

    /// Regression: registry-mode build.gradle.kts must emit the full Maven
    /// coordinate (`groupId:artifactId:version`) for the published Android AAR,
    /// not just the artifact name. The coordinate is resolved from
    /// `naming::aar_group_id()` and `naming::aar_artifact_id()` so it respects
    /// the `[crates.kotlin_android]` config. Credentials: Maven Central requires
    /// the fully-qualified coordinate (e.g., `dev.sample_core:sample_core-android:5.0.0-rc.1`).
    #[test]
    fn build_gradle_kotlin_android_registry_mode_emits_full_maven_coordinate() {
        let output = render_build_gradle_kotlin_android(
            "sample_crate",
            "dev.sample_crate",
            "5.0.0-rc.1",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Registry,
            false,
        );
        assert!(
            output.contains(r#"implementation("dev.sample_crate:sample_crate-android:5.0.0-rc.1")"#),
            "build.gradle.kts must emit full Maven coordinate with groupId:artifactId:version, got:\n{output}"
        );
    }

    /// Regression: the e2e settings.gradle.kts must declare the
    /// `pluginManagement` block with `google()` and `gradlePluginPortal()` so
    /// Gradle can resolve `com.android.library`. Missing settings.gradle.kts
    /// causes `Plugin [id: 'com.android.library'] was not found` at config time.
    #[test]
    fn settings_gradle_kotlin_android_declares_plugin_repositories() {
        let output = render_settings_gradle_kotlin_android("sample-llm");
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
            output.contains("rootProject.name = \"sample-llm-e2e\""),
            "rootProject.name must be derived from pkg_name, got:\n{output}"
        );
    }

    /// Regression: registry-mode `pkg_name` may be a Maven coordinate
    /// (`group:artifact`) because it's used verbatim as a Gradle dependency
    /// string. Gradle rejects project names containing `:`, so the
    /// emitter must strip the group prefix when deriving `rootProject.name`.
    /// Without sanitization Gradle fails at configuration time with
    /// "The project name '…' must not contain any of the following
    /// characters: [/, \\, :, <, >, \", ?, *, |]".
    #[test]
    fn settings_gradle_kotlin_android_strips_maven_group_from_project_name() {
        let output = render_settings_gradle_kotlin_android("dev.sample_crate:sample-markdown-android");
        assert!(
            output.contains("rootProject.name = \"sample-markdown-android-e2e\""),
            "rootProject.name must strip Maven group prefix, got:\n{output}"
        );
        let project_name_line = output
            .lines()
            .find(|line| line.starts_with("rootProject.name"))
            .expect("rootProject.name line must be emitted");
        assert!(
            !project_name_line.contains(':'),
            "rootProject.name line must not contain Gradle-reserved ':', got:\n{project_name_line}"
        );
    }
}

/// Emit a Kotlin Android test backend stub class for a trait bridge.
///
/// Generates a class implementing `I{TraitName}`. Required methods are overridden
/// with Kotlin-idiomatic defaults. Suspend (async) methods use `suspend fun`.
/// The `name()` function is emitted when a Plugin super-trait is configured.
/// Registration uses `{TraitName}Bridge.register(stub)` (the static object pattern).
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    use crate::backends::kotlin::type_map::KotlinMapper;
    use crate::codegen::defaults::language_defaults;
    use crate::codegen::type_mapper::TypeMapper as _;
    use heck::{ToLowerCamelCase, ToUpperCamelCase};
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Kotlin Android uses I{TraitName} as the interface.
    let interface_name = format!("I{}", trait_bridge.trait_name);
    // Use the canonical naming helper so both production and e2e emit the same bridge object name.
    let bridge_object = crate::backends::kotlin_android::naming::bridge_object_name(&trait_bridge.trait_name);

    // Prefer the fixture's input "name" field (e.g. "test-extractor") over the
    // fixture id, which is an internal snake_case identifier, not a backend name.
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("kotlin_android");
    let mapper = KotlinMapper;

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name} : {interface_name} {{");

    // Plugin super-trait `name()` function.
    let mut emitted_methods = std::collections::HashSet::new();
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "    override fun name(): String = \"{plugin_name}\"");
        emitted_methods.insert("name".to_string());
    }

    // Required methods only. Trait methods with default implementations are optional
    // for test stubs and should inherit the generated interface default.
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Skip if already emitted (e.g., super-trait name method).
        if emitted_methods.contains(&method.name) {
            continue;
        }
        let method_name = method.name.to_lower_camel_case();

        // Build parameter list with concrete Kotlin types.
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name.to_lower_camel_case(), mapper.map_type(&p.ty)))
            .collect();
        let params_str = params.join(", ");

        let return_type = mapper.map_type(&method.return_type);
        let default_val = defaults.emit_default(&method.return_type);

        if method.is_async {
            let _ = writeln!(
                setup,
                "    override suspend fun {method_name}({params_str}): {return_type} = {default_val}"
            );
        } else {
            let _ = writeln!(
                setup,
                "    override fun {method_name}({params_str}): {return_type} = {default_val}"
            );
        }
        emitted_methods.insert(method.name.clone());
    }

    let _ = writeln!(setup, "}}");

    // Registration: `{TraitName}Bridge.register(stub)` — static object pattern.
    let arg_expr = format!("{class_name}()");
    // Emit a registration comment in the setup block so the caller can see the bridge object.
    let _ = writeln!(setup, "// register via: {bridge_object}.register({class_name}())");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
    }
}

#[cfg(test)]
mod test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        }
    }

    /// Verify that no sample_core-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn kotlin_android_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        // The bridge object is "TestTraitBridge" not "SampleCrateBridge"
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "class name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("ITestTrait"),
            "class must implement interface derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("TestTraitBridge"),
            "setup block must reference the bridge object derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("processItem"),
            "required method must be emitted in camelCase, got:\n{output}"
        );
    }

    fn make_param(name: &str, ty: crate::core::ir::TypeRef) -> crate::core::ir::ParamDef {
        crate::core::ir::ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
        }
    }

    fn make_method_with_params(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![
                make_param("content", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            return_type: TypeRef::Named("ExtractionResult".to_string()),
            is_async: true,
            is_static: false,
            error_type: Some("anyhow::Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Verify params use concrete Kotlin types (not `Any`) and return type is concrete.
    #[test]
    fn kotlin_android_stub_uses_typed_params_not_any() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method_with_params("extractBytes", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains(": Any"),
            "param type must not be `Any`, got:\n{output}"
        );
        assert!(
            output.contains("content: ByteArray"),
            "bytes param must map to ByteArray in Kotlin, got:\n{output}"
        );
        assert!(
            output.contains("mimeType: String"),
            "string param must map to String in Kotlin, got:\n{output}"
        );
        assert!(
            output.contains("): ExtractionResult"),
            "return type must be concrete not Any, got:\n{output}"
        );
    }

    /// Verify that `fixture.input["name"]` is used as the plugin name when present.
    #[test]
    fn kotlin_android_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "name": "my-backend-name" });

        let emission = emit_test_backend(&bridge, &methods, &fixture);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.name, got:\n{output}"
        );
    }
}
