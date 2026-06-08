//! Kotlin Android e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates host-JVM tests that validate the AAR-bundled Java facade and Kotlin wrapper
//! via JNA against the generated FFI library. Tests are emitted to `e2e/kotlin_android/src/test/kotlin/`
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

/// Return the gradle-wrapper.jar content as base64-encoded string.
/// This JAR is the official Gradle 8.5 wrapper JAR from the Gradle project.
/// It is stored as base64 so it can be embedded as a string and decoded at write time.
fn get_gradle_wrapper_jar_base64() -> String {
    // Gradle 8.5 wrapper JAR (42KB) encoded as base64.
    // Source: https://raw.githubusercontent.com/gradle/gradle/v8.5.0/gradle/wrapper/gradle-wrapper.jar
    include_str!("../../../assets/gradle-wrapper-8.5.jar.b64")
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

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
    // In local mode: wire workspace sources directly via sourceSets so no
    // publish step is needed during development.
    let (source_sets_block, artifact_dep) = if dep_mode == crate::e2e::config::DependencyMode::Registry {
        let artifact = format!(
            r#"    // Published Android AAR from Maven Central (verifies artifact resolution)
    implementation("{maven_coordinate}")"#
        );
        (String::new(), artifact)
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
        (src_sets.to_string(), String::new())
    };

    // JUnit 5 test discovery requires useJUnitPlatform() in both local and
    // registry modes. In local mode, also wire JNA for native library loading
    // from the workspace target directory.
    // In registry mode, emit a verifyAarPublished task that downloads and inspects
    // the published AAR without loading JNI on the host JVM.
    let tasks_block = if dep_mode == crate::e2e::config::DependencyMode::Registry {
        format!(
            r#"tasks.register("verifyAarPublished") {{
    description = "Verify the published Android AAR contains jni and classes.jar"
    doLast {{
        val aarCoord = "{maven_coordinate}"
        val (groupId, artifactId, version) = run {{
            val parts = aarCoord.split(':')
            Triple(parts[0], parts[1], parts[2])
        }}
        val aarFileName = "${{artifactId}}-${{version}}.aar"
        val mavenUrl = "https://repo1.maven.org/maven2/${{groupId.replace('.', '/')}}/${{artifactId}}/${{version}}/${{aarFileName}}"
        val aarFile = layout.buildDirectory.file("tmp/${{aarFileName}}").get().asFile

        println("Downloading AAR from Maven Central: ${{mavenUrl}}")
        aarFile.parentFile.mkdirs()

        val connection = URL(mavenUrl).openConnection() as HttpURLConnection
        connection.requestMethod = "GET"
        connection.connect()

        if (connection.responseCode != 200) {{
            throw GradleException("Failed to download AAR: HTTP ${{connection.responseCode}}")
        }}

        connection.inputStream.use {{ input ->
            aarFile.outputStream().use {{ output ->
                input.copyTo(output)
            }}
        }}

        println("Verifying AAR contents...")
        ZipFile(aarFile).use {{ zip ->
            val entries = zip.entries().toList()
            val hasJni = entries.any {{ it.name.startsWith("jni/") }}
            val hasClasses = entries.any {{ it.name == "classes.jar" }}

            if (!hasJni) {{
                throw GradleException("AAR missing jni directory")
            }}
            if (!hasClasses) {{
                throw GradleException("AAR missing classes.jar")
            }}

            val abiDirs = entries
                .filter {{ it.name.startsWith("jni/") }}
                .map {{ it.name.substringAfter("jni/").substringBefore("/") }}
                .filter {{ it.isNotEmpty() }}
                .distinct()

            println("  + jni: YES")
            println("  + classes.jar: YES")
            println("  + Android ABIs: " + abiDirs.sorted().joinToString(", "))
            println("\nAAR verification PASSED!")
        }}
    }}
}}

tasks.withType<Test> {{
    useJUnitPlatform()
    dependsOn("verifyAarPublished")
}}"#,
            maven_coordinate = maven_coordinate
        )
    } else {
        r#"tasks.withType<Test> {
    useJUnitPlatform()

    // Resolve the native library location (e.g., ../../target/release)
    val libPath = System.getProperty("kb.lib.path") ?: "${rootDir}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)

    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/
    workingDir = file("${rootDir}/../../test_documents")
}"#
        .to_string()
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
        r#"import java.net.HttpURLConnection
import java.net.URL
import java.util.zip.ZipFile
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
    // Set JVM target for compilation. gradle.properties enables auto-detection
    // of host JDK installations so Gradle uses the available JDK version on the
    // build machine, preventing provisioning failures when the target version is not installed.
    jvmToolchain({jvm_target})
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

plugins {{
    id("org.gradle.toolchains.foojay-resolver-convention") version "0.7.0"
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

/// Render `gradle/wrapper/gradle-wrapper.properties` for the gradle wrapper.
///
/// Points to a Gradle distribution URL. This file is downloaded/cached by the
/// wrapper scripts on first invocation.
fn render_gradle_wrapper_properties() -> String {
    // Use Gradle 8.13 (required by AGP 8.13.0+). Gradle wrapper scripts automatically
    // download and cache the distribution.
    const GRADLE_VERSION: &str = "8.13";
    format!(
        r#"distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-{GRADLE_VERSION}-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
"#
    )
}

/// Render `gradle.properties` for the e2e project.
///
/// Configures Gradle toolchain behavior to allow auto-detection of the host JDK
/// and fall back to auto-downloading from Adoptium when the requested
/// `jvmToolchain(N)` is not available. This prevents build failures on
/// hosts that don't have a specific JDK version installed.
fn render_gradle_properties() -> String {
    r#"# Generated by alef. Do not edit by hand.

# Allow Gradle to auto-detect JDK installations when the requested
# toolchain version is not available. This prevents build failures on
# hosts with only newer or older JDK versions installed.
org.gradle.java.installations.auto-detect=true

# Configure Adoptium (Eclipse Temurin) as the download repository for
# missing JDK toolchains. When jvmToolchain(17) is requested but JDK 17
# is not found locally, Gradle will attempt to download it from this repo.
org.gradle.jvm.toolchain.download.repository=adoptium

# Increase heap for large multi-project builds.
org.gradle.jvmargs=-Xmx4g
"#
    .to_string()
}

/// Unix shell script for gradle wrapper (`gradlew`).
///
/// Bootstraps gradle-wrapper.jar download from the URL in
/// gradle-wrapper.properties on first invocation. Shebang triggers 0755
/// chmod in the file writer.
const GRADLE_WRAPPER_UNIX: &str = r#"#!/bin/sh

#
# Copyright 2015 the original author or authors.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#

##############################################################################
##
##  Gradle start up script for UN*X
##
##############################################################################

# Attempt to set APP_HOME
# Resolve links: $0 may be a link
PRG="$0"
# Need this for relative symlinks.
while [ -h "$PRG" ] ; do
    ls -ld "$PRG"
    link=`expr "$PRG" : '.*-> \(.*\)$'`
    if expr "$link" : '/.*' > /dev/null; then
        PRG="$link"
    else
        PRG=`dirname "$PRG"`"/$link"
    fi
done
SAVED="`pwd`"
cd "`dirname "$PRG"`/" >/dev/null
APP_HOME="`pwd -P`"
cd "$SAVED" >/dev/null

APP_NAME="Gradle"
APP_BASE_NAME=`basename "$0"`

# Add default JVM options here. You can also use JAVA_OPTS and GRADLE_OPTS to pass JVM options to this script.
DEFAULT_JVM_OPTS='"-Xmx64m" "-Xms64m"'

# Use the maximum available, or set MAX_FD != -1 to use that value.
MAX_FD="maximum"

warn () {
    echo "$*"
} >&2

die () {
    echo
    echo "$*"
    echo
    exit 1
} >&2

# OS specific support (must be 'true' or 'false').
cygwin=false
msys=false
darwin=false
nonstop=false
case "`uname`" in
  CYGWIN* )
    cygwin=true
    ;;
  Darwin* )
    darwin=true
    ;;
  MSYS* | MINGW* )
    msys=true
    ;;
  NONSTOP* )
    nonstop=true
    ;;
esac

CLASSPATH=$APP_HOME/gradle/wrapper/gradle-wrapper.jar

# Determine the Java command to use to start the JVM.
if [ -n "$JAVA_HOME" ] ; then
    if [ -x "$JAVA_HOME/jre/sh/java" ] ; then
        # IBM's JDK on AIX uses strange locations for the executables
        JAVACMD="$JAVA_HOME/jre/sh/java"
    else
        JAVACMD="$JAVA_HOME/bin/java"
    fi
    if [ ! -x "$JAVACMD" ] ; then
        die "ERROR: JAVA_HOME is set to an invalid directory: $JAVA_HOME

Please set the JAVA_HOME variable in your environment to match the
location of your Java installation."
    fi
else
    JAVACMD="java"
    which java >/dev/null 2>&1 || die "ERROR: JAVA_HOME is not set and no 'java' command could be found in your PATH.

Please set the JAVA_HOME variable in your environment to match the
location of your Java installation."
fi

# Increase the maximum file descriptors if we can.
if [ "$cygwin" = "false" -a "$msys" = "false" ] && command -v ulimit > /dev/null ; then
    if [ "$nonstop" = "false" ] ; then
        # Try setting the maximum allowed open files if we know how to.
        # Linux sets the default to 1024.
        if [ -n "$MAX_FD" -a \( "$MAX_FD" = "maximum" -o "$MAX_FD" = "max" \) ] ; then
            MAX_FD_LIMIT=`ulimit -H -n`
            if [ $? -eq 0 ] ; then
                if [ "$MAX_FD" = "maximum" -o "$MAX_FD" = "max" ] ; then
                    MAX_FD="$MAX_FD_LIMIT"
                fi
                ulimit -n $MAX_FD
                if [ $? -ne 0 ] ; then
                    warn "Could not set maximum file descriptor limit: $MAX_FD"
                fi
            else
                warn "Could not query maximum file descriptor limit: $MAX_FD_LIMIT"
            fi
        else
            warn "Max file descriptor limit unknown on this system."
        fi
    else
        warn "Unknown value for MAX_FD: $MAX_FD"
    fi
fi

# For Darwin, add options to specify how the application appears in the dock, menus, etc.
if [ "$darwin" = "true" ] ; then
    GRADLE_OPTS="$GRADLE_OPTS \"-Xdock:name=$APP_NAME\" \"-Xdock:icon=$APP_HOME/media/gradle.icns\""
fi

# For Cygwin or MSYS, switch paths to Windows-native format before running java
if [ "$cygwin" = "true" -o "$msys" = "true" ] ; then
    APP_HOME=`cygpath --path --mixed "$APP_HOME"`
    CLASSPATH=`cygpath --path --mixed "$CLASSPATH"`

    JAVACMD=`cygpath --unix "$JAVACMD"`

    # We build the pattern for arguments to be converted via cygpath
    ROOTDIRSRAW=`find -L / -maxdepth 3 -type d -name gradle 2>/dev/null | head -1`
    if [ -d "$ROOTDIRSRAW" ] ; then
        ROOTDIRS="$ROOTDIRSRAW"
    else
        ROOTDIRS=`dirname "$ROOTDIRSRAW"`
    fi
    SEP=":"
    if [ "$cygwin" = "true" ] ; then
        SEP=";"
    fi
    OURCYGPATTERN="(^($(\\/)|([a-zA-Z]:\\/))\\\\)?([^()\\/| ]*+)(\\\\[^()\\/| |\"]*+)*+$"
    # Add a user-defined pattern to the cygpath arguments
    if [ "$GRADLE_CYGWIN_PATTERN" != "" ] ; then
        OURCYGPATTERN="$OURCYGPATTERN|($GRADLE_CYGWIN_PATTERN)"
    fi
    # Now convert the arguments - kludge to limit ourselves to /bin/sh
    i=0
    for arg in "$@" ; do
        CHECK=`echo "$arg"|egrep -c "$OURCYGPATTERN" -`
        CHECK2=`echo "$arg"|egrep -c "^-"`                                 ### Determine if an option

        if [ $CHECK -ne 0 ] && [ $CHECK2 -eq 0 ] ; then                    ### Added a condition
            eval `echo args$i`=`cygpath --path --ignore --mixed "$arg"`
        else
            eval `echo args$i`="\"$arg\""
        fi
        i=`expr $i + 1`
    done
    case $i in
        0) set -- ;;
        1) set -- "$args0" ;;
        2) set -- "$args0" "$args1" ;;
        3) set -- "$args0" "$args1" "$args2" ;;
        4) set -- "$args0" "$args1" "$args2" "$args3" ;;
        5) set -- "$args0" "$args1" "$args2" "$args3" "$args4" ;;
        6) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" ;;
        7) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" ;;
        8) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" "$args7" ;;
        9) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" "$args7" "$args8" ;;
    esac
fi

# Escape application args
save () {
    for i do printf %s\\n "$i" | sed "s/'/'\\\\''/g;1s/^/'/;\$s/\$/' \\\\/" ; done
    echo " "
}
APP_ARGS=`save "$@"`

# Collect all arguments for the java command, following the shell quoting and substitution rules
eval set -- $DEFAULT_JVM_OPTS $JAVA_OPTS $GRADLE_OPTS "\"-Dorg.gradle.appname=$APP_BASE_NAME\"" -classpath "$CLASSPATH" org.gradle.wrapper.GradleWrapperMain "$APP_ARGS"

exec "$JAVACMD" "$@"
"#;

/// Windows batch script for gradle wrapper (`gradlew.bat`).
const GRADLE_WRAPPER_WINDOWS: &str = r#"@rem
@rem Copyright 2015 the original author or authors.
@rem
@rem Licensed under the Apache License, Version 2.0 (the "License");
@rem you may not use this file except in compliance with the License.
@rem You may obtain a copy of the License at
@rem
@rem      https://www.apache.org/licenses/LICENSE-2.0
@rem
@rem Unless required by applicable law or agreed to in writing, software
@rem distributed under the License is distributed on an "AS IS" BASIS,
@rem WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
@rem See the License for the specific language governing permissions and
@rem limitations under the License.
@rem

@if "%DEBUG%" == "" @echo off
@rem ##########################################################################
@rem
@rem  Gradle startup script for Windows
@rem
@rem ##########################################################################

@rem Set local scope for the variables with windows NT shell
if "%OS%"=="Windows_NT" setlocal

set DIRNAME=%~dp0
if "%DIRNAME%" == "" set DIRNAME=.
set APP_BASE_NAME=%~n0
set APP_HOME=%DIRNAME%

@rem Resolve any "." and ".." in APP_HOME to make it shorter.
for %%i in ("%APP_HOME%") do set APP_HOME=%%~fi

@rem Add default JVM options here. You can also use JAVA_OPTS and GRADLE_OPTS to pass JVM options to this script.
set DEFAULT_JVM_OPTS="-Xmx64m" "-Xms64m"

@rem Find java.exe
if defined JAVA_HOME goto findJavaFromJavaHome

set JAVA_EXE=java.exe
%JAVA_EXE% -version >nul 2>&1
if "%ERRORLEVEL%" == "0" goto execute

echo.
echo ERROR: JAVA_HOME is not set and no 'java' command could be found in your PATH.
echo.
echo Please set the JAVA_HOME variable in your environment to match the
echo location of your Java installation.

goto fail

:findJavaFromJavaHome
set JAVA_HOME=%JAVA_HOME:"=%
set JAVA_EXE=%JAVA_HOME%\bin\java.exe

if exist "%JAVA_EXE%" goto execute

echo.
echo ERROR: JAVA_HOME is set to an invalid directory: %JAVA_HOME%
echo.
echo Please set the JAVA_HOME variable in your environment to match the
echo location of your Java installation.

goto fail

:execute
@rem Setup the command line

set CLASSPATH=%APP_HOME%\gradle\wrapper\gradle-wrapper.jar

@rem Execute Gradle
"%JAVA_EXE%" %DEFAULT_JVM_OPTS% %JAVA_OPTS% %GRADLE_OPTS% "-Dorg.gradle.appname=%APP_BASE_NAME%" -classpath "%CLASSPATH%" org.gradle.wrapper.GradleWrapperMain %*

:end
@endlocal & set ERROR_CODE=%ERRORLEVEL%

if not "%ERROR_CODE%" == "0" goto fail

:fail
exit /b %ERROR_CODE%

:mainEnd
if "%1"=="start" (
	call :startApp
	exit /b
)
call :stopApp
exit /b

:startApp
start "" cmd /k start %APP_HOME%\bin\myApp.bat
exit /b

:stopApp
taskkill /IM myApp.exe /F
exit /b
"#;

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
            "demo-client",
            "dev.sample_crate.samplellm.android",
            "1.0.0",
            "dev.sample_crate:demo-client-android:1.0.0",
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
        let output = render_settings_gradle_kotlin_android("demo-client");
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
            output.contains("rootProject.name = \"demo-client-e2e\""),
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
        let output = render_settings_gradle_kotlin_android("dev.sample_crate:demo-markup-android");
        assert!(
            output.contains("rootProject.name = \"demo-markup-android-e2e\""),
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

    /// Regression: registry-mode build.gradle.kts must emit a `verifyAarPublished`
    /// task that downloads the published AAR from Maven Central and verifies it
    /// contains jni/ and classes.jar. This task serves as a smoke test for
    /// AAR content correctness without requiring JNI loading on the host JVM.
    #[test]
    fn build_gradle_kotlin_android_registry_mode_includes_aar_verification_task() {
        let output = render_build_gradle_kotlin_android(
            "sample_crate",
            "dev.sample_crate",
            "5.0.0-rc.1",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Registry,
            false,
        );
        assert!(
            output.contains("verifyAarPublished"),
            "registry-mode build.gradle.kts must include verifyAarPublished task, got:\n{output}"
        );
        assert!(
            output.contains("startsWith(\"jni/\")"),
            "verifyAarPublished task must check for jni/ directory, got:\n{output}"
        );
        assert!(
            output.contains("classes.jar"),
            "verifyAarPublished task must check for classes.jar, got:\n{output}"
        );
        assert!(
            output.contains("dependsOn(\"verifyAarPublished\")"),
            "Test task must depend on verifyAarPublished, got:\n{output}"
        );
    }

    /// Regression: build.gradle.kts MUST pin the JDK toolchain (`jvmToolchain(17)`).
    /// Without this, `./gradlew test` picks the host JDK; under JDK 25 (Temurin)
    /// the Android Gradle Plugin can't parse the host version string and fails
    /// with `What went wrong: 25.0.2`. Tested in both registry and local modes
    /// since the host JDK affects either mode.
    #[test]
    fn build_gradle_kotlin_android_pins_jvm_toolchain_for_jdk25_host_compat() {
        for dep_mode in [
            crate::e2e::config::DependencyMode::Registry,
            crate::e2e::config::DependencyMode::Local,
        ] {
            let output = render_build_gradle_kotlin_android(
                "sample_crate",
                "dev.sample_crate",
                "5.0.0-rc.1",
                "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                false,
            );
            assert!(
                output.contains("jvmToolchain(17)"),
                "build.gradle.kts ({dep_mode:?}) must pin jvmToolchain(17) so JDK 25 hosts pick up JDK 17 for gradle, got:\n{output}"
            );
        }
    }

    /// Regression: local-mode build.gradle.kts must NOT emit the AAR verification
    /// task — it tests against workspace sources, not published artifacts.
    #[test]
    fn build_gradle_kotlin_android_local_mode_excludes_aar_verification_task() {
        let output = render_build_gradle_kotlin_android(
            "demo-client",
            "dev.sample_crate.samplellm.android",
            "1.0.0",
            "dev.sample_crate:demo-client-android:1.0.0",
            crate::e2e::config::DependencyMode::Local,
            false,
        );
        assert!(
            !output.contains("verifyAarPublished"),
            "local-mode build.gradle.kts must not include verifyAarPublished task, got:\n{output}"
        );
    }

    /// Gradle wrapper properties must reference a valid Gradle version and
    /// point to services.gradle.org distribution URL.
    #[test]
    fn gradle_wrapper_properties_includes_valid_distribution_url() {
        let output = render_gradle_wrapper_properties();
        assert!(
            output.contains("services.gradle.org/distributions/gradle-"),
            "gradle-wrapper.properties must reference services.gradle.org distribution, got:\n{output}"
        );
        assert!(
            output.contains("-bin.zip"),
            "gradle-wrapper.properties must reference -bin.zip distribution, got:\n{output}"
        );
        assert!(
            output.contains("distributionBase=GRADLE_USER_HOME"),
            "gradle-wrapper.properties must set distributionBase, got:\n{output}"
        );
    }

    /// The gradle wrapper unix script must contain shebang and valid shell syntax.
    #[test]
    fn gradle_wrapper_unix_script_is_valid_shell() {
        assert!(
            GRADLE_WRAPPER_UNIX.starts_with("#!/bin/sh"),
            "gradlew must start with #!/bin/sh shebang"
        );
        assert!(GRADLE_WRAPPER_UNIX.contains("CLASSPATH"), "gradlew must set CLASSPATH");
        assert!(
            GRADLE_WRAPPER_UNIX.contains("org.gradle.wrapper.GradleWrapperMain"),
            "gradlew must invoke GradleWrapperMain"
        );
    }

    /// The gradle wrapper windows script must be valid batch syntax.
    #[test]
    fn gradle_wrapper_windows_script_is_valid_batch() {
        assert!(
            GRADLE_WRAPPER_WINDOWS.starts_with("@rem"),
            "gradlew.bat must start with @rem comment"
        );
        assert!(
            GRADLE_WRAPPER_WINDOWS.contains("java.exe"),
            "gradlew.bat must reference java.exe"
        );
        assert!(
            GRADLE_WRAPPER_WINDOWS.contains("org.gradle.wrapper.GradleWrapperMain"),
            "gradlew.bat must invoke GradleWrapperMain"
        );
    }

    /// gradle-wrapper.jar must be emitted as base64-encoded content.
    /// The file writer will detect the .jar extension and decode it automatically.
    #[test]
    fn gradle_wrapper_jar_is_base64_encoded() {
        let jar_b64 = get_gradle_wrapper_jar_base64();
        // Base64 content should start with the ZIP file magic bytes encoded as base64.
        // ZIP files start with PK (0x504B), which encodes to "UEsD" in base64.
        assert!(
            jar_b64.starts_with("UEsD"),
            "gradle-wrapper.jar base64 must start with encoded ZIP magic bytes 'UEsD', got:\n{}",
            &jar_b64[..std::cmp::min(50, jar_b64.len())]
        );
        // Base64 should be valid (no newlines in the embedded constant).
        assert!(
            !jar_b64.contains('\n'),
            "gradle-wrapper.jar base64 must not contain newlines"
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
        teardown_block: String::new(),
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
            assertion_recipes: vec![],
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
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn make_method_with_params(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![
                make_param("content", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            return_type: TypeRef::Named("ProcessingResult".to_string()),
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
            output.contains("): ProcessingResult"),
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
