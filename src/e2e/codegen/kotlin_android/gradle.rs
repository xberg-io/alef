use crate::core::template_versions::{maven, toolchain};

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
pub(super) fn render_build_gradle_kotlin_android(
    kotlin_pkg_id: &str,
    maven_coordinate: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    jni_lib_name: &str,
    jni_crate_path: &str,
) -> String {
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let android_gradle_plugin = maven::ANDROID_GRADLE_PLUGIN;
    // AGP 9.0+ ships built-in Kotlin support and rejects the explicit
    // `kotlin("android")` plugin; emit it only for AGP < 9 (see the package
    // build.gradle emitter for the full rationale).
    let agp_major: u32 = android_gradle_plugin
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or(0);
    let kotlin_android_plugin_line = if agp_major >= 9 {
        String::new()
    } else {
        format!("\n    kotlin(\"android\") version \"{kotlin_plugin}\"")
    };
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
    // The JUnit Platform launcher must be on the test classpath at both compile
    // and runtime: the Gradle Test Executor loads it at runtime to discover and
    // launch JUnit Platform tests, and the generated `MockServerListener`
    // implements the `LauncherSessionListener` SPI, referencing
    // `org.junit.platform.launcher.{LauncherSession, LauncherSessionListener}`
    // as compile-time symbols. Scoping it `testRuntimeOnly` keeps it off the
    // compile classpath and fails Kotlin compilation with
    // "Unresolved reference 'launcher'", so use `testImplementation` (a superset
    // of `testRuntimeOnly` that covers both compile and runtime).
    let launcher_dep = format!(r#"    testImplementation("org.junit.platform:junit-platform-launcher:{junit}")"#);

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
    // In both modes, emit buildHostJni and copyHostJni tasks for JVM unit tests
    // that load System.loadLibrary("{jni_lib_name}").
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

// Build host JNI library for JVM unit tests (macOS/Linux/Windows).
// The generated Kotlin Bridge object calls System.loadLibrary("{jni_lib_name}") for JVM
// unit tests running on developer machines. This task builds the host-platform binary
// and stages it into src/test/resources/host-jni/<platform>/ for the test loader.
// Set alef.skipHostJni=true to disable this (e.g., in CI where only AAR validation is needed).
tasks.register("buildHostJni", Exec::class) {{
    if (project.properties["alef.skipHostJni"] != "true") {{
        val jniCargoPath = "{jni_crate_path}/Cargo.toml"
        description = "Build host-platform JNI library from {jni_crate_path}"
        commandLine("cargo", "build", "--release", "--manifest-path", jniCargoPath)
        errorOutput = System.err
    }} else {{
        description = "Build host JNI (disabled via alef.skipHostJni=true)"
        commandLine("true")
    }}
}}

tasks.register("copyHostJni", Copy::class) {{
    if (project.properties["alef.skipHostJni"] != "true") {{
        description = "Copy host JNI library to test resources"
        dependsOn("buildHostJni")

        val hostPlatform = if (System.getProperty("os.name").lowercase().contains("mac")) {{
            "darwin"
        }} else if (System.getProperty("os.name").lowercase().contains("win")) {{
            "windows"
        }} else {{
            "linux"
        }}
        val libName = when (hostPlatform) {{
            "darwin" -> "lib{jni_lib_name}.dylib"
            "windows" -> "{jni_lib_name}.dll"
            else -> "lib{jni_lib_name}.so"
        }}

        // Cargo builds to the workspace target directory by default, even when
        // --manifest-path points at a member crate. The previous
        // `if (workspaceTarget.exists()) ... else crateTarget` dual-path was
        // evaluated at gradle configuration time, before `cargo build` finished
        // or before the workspace target dir existed, so the glob could match
        // zero files and the test runtime would fail with `UnsatisfiedLinkError`
        // at static-init time. Always read from the workspace target.
        val workspaceTarget = file("../../target/release")

        from(workspaceTarget) {{
            include(libName)
        }}
        into(layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform"))
    }}
}}

tasks.withType<Test> {{
    useJUnitPlatform()
    dependsOn("verifyAarPublished")
    if (project.properties["alef.skipHostJni"] != "true") {{
        val hostPlatform = if (System.getProperty("os.name").lowercase().contains("mac")) {{
            "darwin"
        }} else if (System.getProperty("os.name").lowercase().contains("win")) {{
            "windows"
        }} else {{
            "linux"
        }}
        systemProperty(
            "java.library.path",
            project.layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform").asFile.absolutePath
        )
        dependsOn("copyHostJni")
    }}
}}

tasks.matching {{ it.name.startsWith("processDebug") || it.name.startsWith("processRelease") }}.configureEach {{
    if (project.properties["alef.skipHostJni"] != "true" && name.contains("UnitTestJavaRes")) {{
        dependsOn("copyHostJni")
    }}
}}"#,
            maven_coordinate = maven_coordinate,
            jni_crate_path = jni_crate_path,
            jni_lib_name = jni_lib_name,
        )
    } else {
        format!(
            r#"// Build host JNI library for JVM unit tests (macOS/Linux/Windows).
// The generated Kotlin Bridge object calls System.loadLibrary("{jni_lib_name}") for JVM
// unit tests running on developer machines. This task builds the host-platform binary
// and stages it into src/test/resources/host-jni/<platform>/ for the test loader.
// Set alef.skipHostJni=true to disable this (e.g., in CI where only source-set validation is needed).
tasks.register("buildHostJni", Exec::class) {{
    if (project.properties["alef.skipHostJni"] != "true") {{
        val jniCargoPath = "{jni_crate_path}/Cargo.toml"
        description = "Build host-platform JNI library from {jni_crate_path}"
        commandLine("cargo", "build", "--release", "--manifest-path", jniCargoPath)
        errorOutput = System.err
    }} else {{
        description = "Build host JNI (disabled via alef.skipHostJni=true)"
        commandLine("true")
    }}
}}

tasks.register("copyHostJni", Copy::class) {{
    if (project.properties["alef.skipHostJni"] != "true") {{
        description = "Copy host JNI library to test resources"
        dependsOn("buildHostJni")

        val hostPlatform = if (System.getProperty("os.name").lowercase().contains("mac")) {{
            "darwin"
        }} else if (System.getProperty("os.name").lowercase().contains("win")) {{
            "windows"
        }} else {{
            "linux"
        }}
        val libName = when (hostPlatform) {{
            "darwin" -> "lib{jni_lib_name}.dylib"
            "windows" -> "{jni_lib_name}.dll"
            else -> "lib{jni_lib_name}.so"
        }}

        // Cargo builds to the workspace target directory by default, even when
        // --manifest-path points at a member crate. The previous
        // `if (workspaceTarget.exists()) ... else crateTarget` dual-path was
        // evaluated at gradle configuration time, before `cargo build` finished
        // or before the workspace target dir existed, so the glob could match
        // zero files and the test runtime would fail with `UnsatisfiedLinkError`
        // at static-init time. Always read from the workspace target.
        val workspaceTarget = file("../../target/release")

        from(workspaceTarget) {{
            include(libName)
        }}
        into(layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform"))
    }}
}}

tasks.withType<Test> {{
    useJUnitPlatform()

    // Resolve the native library location (e.g., ../../target/release)
    val libPath = System.getProperty("kb.lib.path") ?: "${{rootDir}}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)

    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/
    workingDir = file("${{rootDir}}/../../test_documents")

    if (project.properties["alef.skipHostJni"] != "true") {{
        val hostPlatform = if (System.getProperty("os.name").lowercase().contains("mac")) {{
            "darwin"
        }} else if (System.getProperty("os.name").lowercase().contains("win")) {{
            "windows"
        }} else {{
            "linux"
        }}
        systemProperty(
            "java.library.path",
            project.layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform").asFile.absolutePath
        )
        dependsOn("copyHostJni")
    }}
}}

tasks.matching {{ it.name.startsWith("processDebug") || it.name.startsWith("processRelease") }}.configureEach {{
    if (project.properties["alef.skipHostJni"] != "true" && name.contains("UnitTestJavaRes")) {{
        dependsOn("copyHostJni")
    }}
}}"#,
            jni_crate_path = jni_crate_path,
            jni_lib_name = jni_lib_name,
        )
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
    id("com.android.library") version "{android_gradle_plugin}"{kotlin_android_plugin_line}
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
pub(super) fn render_settings_gradle_kotlin_android(pkg_name: &str) -> String {
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
    id("org.gradle.toolchains.foojay-resolver-convention") version "1.0.0"
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

/// Render `gradle.properties` for the e2e project.
///
/// Configures Gradle toolchain behavior to allow auto-detection of the host JDK
/// and fall back to auto-downloading from Adoptium when the requested
/// `jvmToolchain(N)` is not available. This prevents build failures on
/// hosts that don't have a specific JDK version installed.
pub(super) fn render_gradle_properties() -> String {
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
            "dev.sample_crate.samplellm.android",
            "dev.sample_crate:demo-client-android:1.0.0",
            crate::e2e::config::DependencyMode::Local,
            "demo_client_jni",
            "../../crates/demo-client-jni",
        );
        assert!(
            output.contains("jackson-module-kotlin"),
            "build.gradle.kts must depend on jackson-module-kotlin, got:\n{output}"
        );
    }

    /// Regression: build.gradle.kts must always put the JUnit Platform launcher
    /// on the test classpath. The Gradle Test Executor loads it at runtime to
    /// discover and launch JUnit Platform tests, and the generated
    /// `MockServerListener` references `LauncherSession`/`LauncherSessionListener`
    /// at compile time. It must be `testImplementation` (covering both compile
    /// and runtime) and present in both dependency modes, regardless of whether a
    /// mock server is needed.
    #[test]
    fn build_gradle_kotlin_android_always_includes_junit_platform_launcher() {
        for dep_mode in [
            crate::e2e::config::DependencyMode::Registry,
            crate::e2e::config::DependencyMode::Local,
        ] {
            let output = render_build_gradle_kotlin_android(
                "dev.sample_crate",
                "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                "sample_crate_jni",
                "../../crates/sample_crate-jni",
            );
            assert!(
                output.contains(r#"testImplementation("org.junit.platform:junit-platform-launcher:"#),
                "build.gradle.kts ({dep_mode:?}) must declare junit-platform-launcher as testImplementation, got:\n{output}"
            );
            assert!(
                output.contains("useJUnitPlatform()"),
                "build.gradle.kts ({dep_mode:?}) must call useJUnitPlatform(), got:\n{output}"
            );
        }
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
            "dev.sample_crate",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Registry,
            "sample_crate_jni",
            "../../crates/sample_crate-jni",
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
            "dev.sample_crate",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Registry,
            "sample_crate_jni",
            "../../crates/sample_crate-jni",
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
                "dev.sample_crate",
                "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                "sample_crate_jni",
                "../../crates/sample_crate-jni",
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
            "dev.sample_crate.samplellm.android",
            "dev.sample_crate:demo-client-android:1.0.0",
            crate::e2e::config::DependencyMode::Local,
            "demo_client_jni",
            "../../crates/demo-client-jni",
        );
        assert!(
            !output.contains("verifyAarPublished"),
            "local-mode build.gradle.kts must not include verifyAarPublished task, got:\n{output}"
        );
    }

    /// Regression: both local and registry modes must emit buildHostJni and copyHostJni
    /// tasks so that JVM unit tests can load System.loadLibrary("{jni_lib_name}").
    /// Without these tasks, gradle test fails with UnsatisfiedLinkError on the host JVM.
    #[test]
    fn build_gradle_kotlin_android_includes_host_jni_tasks() {
        for dep_mode in [
            crate::e2e::config::DependencyMode::Registry,
            crate::e2e::config::DependencyMode::Local,
        ] {
            let output = render_build_gradle_kotlin_android(
                "dev.sample_crate",
                "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                "sample_crate_jni",
                "../../crates/sample_crate-jni",
            );

            assert!(
                output.contains(r#"tasks.register("buildHostJni", Exec::class)"#),
                "build.gradle.kts ({dep_mode:?}) must include buildHostJni task registration, got:\n{output}"
            );
            assert!(
                output.contains(r#"tasks.register("copyHostJni", Copy::class)"#),
                "build.gradle.kts ({dep_mode:?}) must include copyHostJni task registration, got:\n{output}"
            );
            assert!(
                output.contains("java.library.path"),
                "build.gradle.kts ({dep_mode:?}) must set java.library.path for the Test task, got:\n{output}"
            );
            assert!(
                output.contains(r#"src/test/resources/host-jni"#),
                "build.gradle.kts ({dep_mode:?}) must reference src/test/resources/host-jni, got:\n{output}"
            );
        }
    }

    /// Regression: buildHostJni task must reference the JNI crate path
    /// and build the JNI library for the host platform.
    #[test]
    fn build_gradle_kotlin_android_build_host_jni_uses_parameterized_jni_crate_path() {
        let output = render_build_gradle_kotlin_android(
            "dev.sample_crate",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Local,
            "sample_crate_jni",
            "../../crates/sample_crate-jni",
        );

        assert!(
            output.contains("../../crates/sample_crate-jni/Cargo.toml"),
            "buildHostJni must pass the parameterized JNI crate path to cargo build, got:\n{output}"
        );
        assert!(
            output.contains(r#"commandLine("cargo", "build", "--release", "--manifest-path", jniCargoPath)"#),
            "buildHostJni must invoke cargo build with --release flag, got:\n{output}"
        );
    }

    /// Regression: copyHostJni task must reference the parameterized JNI library name
    /// when mapping platform-specific filenames (libsample_crate_jni.dylib, etc).
    #[test]
    fn build_gradle_kotlin_android_copy_host_jni_uses_parameterized_jni_lib_name() {
        let output = render_build_gradle_kotlin_android(
            "dev.sample_crate",
            "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            crate::e2e::config::DependencyMode::Registry,
            "sample_crate_jni",
            "../../crates/sample_crate-jni",
        );

        assert!(
            output.contains("libsample_crate_jni.dylib"),
            "copyHostJni must emit macOS library name with parameterized JNI lib name, got:\n{output}"
        );
        assert!(
            output.contains("sample_crate_jni.dll"),
            "copyHostJni must emit Windows library name with parameterized JNI lib name, got:\n{output}"
        );
        assert!(
            output.contains("libsample_crate_jni.so"),
            "copyHostJni must emit Linux library name with parameterized JNI lib name, got:\n{output}"
        );
    }
}
