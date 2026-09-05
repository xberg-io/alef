use crate::core::template_versions::{maven, toolchain};

/// Inputs for [`render_build_gradle_kotlin_android`], grouped into one struct so the
/// call stays under clippy's argument threshold. Mirrors the precedent in
/// `src/cli/commands/snippets.rs::ConfiguredCheckInputs`.
pub(super) struct KotlinAndroidBuildGradleInputs<'a> {
    pub(super) kotlin_pkg_id: &'a str,
    pub(super) maven_coordinate: &'a str,
    pub(super) dep_mode: crate::e2e::config::DependencyMode,
    pub(super) jni_lib_name: &'a str,
    pub(super) jni_crate_path: &'a str,
    pub(super) e2e_env: &'a std::collections::BTreeMap<String, String>,
    pub(super) capsule_types: &'a std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    pub(super) test_documents_path: &'a str,
}

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
pub(super) fn render_build_gradle_kotlin_android(inputs: &KotlinAndroidBuildGradleInputs<'_>) -> String {
    let kotlin_pkg_id = inputs.kotlin_pkg_id;
    let maven_coordinate = inputs.maven_coordinate;
    let dep_mode = inputs.dep_mode;
    let jni_lib_name = inputs.jni_lib_name;
    let jni_crate_path = inputs.jni_crate_path;
    let e2e_env = inputs.e2e_env;
    let capsule_types = inputs.capsule_types;
    let test_documents_path = inputs.test_documents_path;

    // Forward `[crates.e2e.env]` vars into the Gradle test worker's *process*
    // environment via `environment(...)`. The worker is a forked JVM, and the
    // JNI-loaded native library reads these via libc `getenv` — e.g. a downstream
    // service allowlist env var that lets URL fixtures reach the loopback mock
    // server. Java has no portable way to mutate `environ` in-process (unlike
    // C#'s `Environment.SetEnvironmentVariable` or Elixir's `set_env` NIF), so
    // it must be set at worker-fork time. `e2e_env` is a `BTreeMap`, so this already
    // iterates in key order for deterministic output. ~keep
    let test_env_block = {
        let mut block = String::new();
        for (key, value) in e2e_env {
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            block.push_str(&format!("\n    environment(\"{}\", \"{}\")", esc(key), esc(value)));
        }
        block
    };
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
    // jackson-annotations dropped patch-version releases after 2.19.x (2.20, 2.21, 2.22,
    // ... -- no third component), unlike jackson-databind/datatype-jdk8/module-kotlin, which
    // still publish `major.minor.patch`. `JACKSON_E2E` tracks the latter scheme; reusing it
    // for jackson-annotations resolves a coordinate (e.g. `2.22.2`) that was never published,
    // so `:generateDebugUnitTestStubRFile` (or any task needing the test classpath resolved)
    // fails with "Could not find com.fasterxml.jackson.core:jackson-annotations:2.22.2"
    // before Kotlin compilation ever starts. `scaffold::languages::kotlin` (the real package
    // build.gradle.kts generator) already gets this right via the dedicated
    // `JACKSON_ANNOTATIONS` constant; this e2e path had its own copy of the dependency list
    // and never picked it up. ~keep
    let jackson_annotations = maven::JACKSON_ANNOTATIONS;
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
    useJUnitPlatform(){test_env_block}
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
            test_env_block = test_env_block,
        )
    } else {
        // Resolve fixture paths (e.g. "docx/fake.docx") against the configured
        // test_documents dir. Guard on existence: Gradle test workers fail to fork if
        // `workingDir` points at a directory that does not exist -- see
        // `gradle/guarded_working_dir.kt.jinja` for the full rationale. Mirrors the
        // plain-Kotlin generator's guard (`kotlin::project::render_build_gradle`).
        let guarded_working_dir = crate::e2e::template_env::render(
            "gradle/guarded_working_dir.kt.jinja",
            minijinja::context! { test_documents_path => test_documents_path },
        );
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
    useJUnitPlatform(){test_env_block}

    // Resolve the native library location (e.g., ../../target/release)
    val libPath = System.getProperty("kb.lib.path") ?: "${{rootDir}}/../../target/release"
    systemProperty("jna.library.path", libPath)

{guarded_working_dir}
    if (project.properties["alef.skipHostJni"] != "true") {{
        val hostPlatform = if (System.getProperty("os.name").lowercase().contains("mac")) {{
            "darwin"
        }} else if (System.getProperty("os.name").lowercase().contains("win")) {{
            "windows"
        }} else {{
            "linux"
        }}
        val hostedPath = project.layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform").asFile.absolutePath
        systemProperty("java.library.path", "$hostedPath:$libPath")
        dependsOn("copyHostJni")
    }} else {{
        systemProperty("java.library.path", libPath)
    }}
}}

tasks.matching {{ it.name.startsWith("processDebug") || it.name.startsWith("processRelease") }}.configureEach {{
    if (project.properties["alef.skipHostJni"] != "true" && name.contains("UnitTestJavaRes")) {{
        dependsOn("copyHostJni")
    }}
}}"#,
            jni_crate_path = jni_crate_path,
            jni_lib_name = jni_lib_name,
            test_env_block = test_env_block,
            guarded_working_dir = guarded_working_dir,
        )
    };

    // Local mode compiles `packages/kotlin-android`'s wrapper sources directly into this
    // test project's `test` sourceSet (see `source_sets_block` above), so any capsule-type
    // host package those sources import (e.g. `io.github.treesitter.ktreesitter.Language`
    // for a `[crates.kotlin_android.capsule_types.Language]` passthrough) must be on this
    // project's own test classpath too -- `render_build_gradle_kotlin_android` had no
    // capsule awareness at all, so the copied sources referenced a package this file never
    // declared and every local-mode e2e build failed with "Unresolved reference 'github'"
    // (or whatever the host package's top segment is) before a single test ran. Registry
    // mode does not need this: the published AAR declares its capsule dependency as `api`
    // (see `backends::kotlin_android::gen_build_gradle`), so it already resolves
    // transitively through the `implementation(mavenCoordinate)` dependency below. ~keep
    let capsule_test_deps: String = {
        let mut deps: Vec<(String, String)> = if dep_mode == crate::e2e::config::DependencyMode::Local {
            capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .map(|cap| (cap.package.clone(), cap.package_version.clone()))
                .collect()
        } else {
            Vec::new()
        };
        deps.sort();
        deps.dedup();
        deps.iter()
            .map(|(coord, ver)| format!("\n    testImplementation(\"{coord}:{ver}\")"))
            .collect()
    };

    // Test dependencies are always needed for host-JVM tests (both Local and Registry modes).
    let test_deps = format!(
        r#"    // Jackson for JSON assertion helpers
    testImplementation("com.fasterxml.jackson.core:jackson-annotations:{jackson_annotations}")
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
    testImplementation("net.java.dev.jna:jna:{jna}"){capsule_test_deps}
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
    let marker = crate::core::hash::SELF_MARKING_HEADER_LINE;
    format!(
        r#"{marker}

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
