//! `build.gradle.kts` emitter — full Android library project with
//! vanniktech maven-publish, ktlint hooks, bundled-Java-facade source set wiring,
//! and optional host JNI binary build for JVM unit tests.

use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions::{maven, toolchain};

use crate::backends::kotlin_android::naming::{
    aar_artifact_id, aar_group_id, compile_sdk, jvm_target, min_sdk, namespace,
};
use crate::scaffold::{parse_author, scaffold_meta, xml_escape};

/// Emit `build.gradle.kts` for the generated AAR module.
///
/// Note on plugin compatibility: AGP 8.x requires an explicit
/// `kotlin("android")` plugin application, while AGP 9.0+ ships with
/// built-in Kotlin support and rejects re-application of
/// `org.jetbrains.kotlin.android`. The emitted plugins block keys the
/// `kotlin("android")` line off the AGP major version derived from the
/// pin, so moving the pin across the 8→9 boundary stays correct
/// automatically. The `kotlin {}` compiler-options block and `JvmTarget`
/// import remain valid under AGP 9 built-in Kotlin.
pub fn emit(config: &ResolvedCrateConfig) -> String {
    let kotlin_version = maven::KOTLIN_JVM_PLUGIN;
    let android_gradle_plugin = maven::ANDROID_GRADLE_PLUGIN;
    // AGP 9.0+ ships built-in Kotlin support and rejects re-application of the
    // `org.jetbrains.kotlin.android` plugin; AGP 8.x requires the explicit line.
    // Emit it only for AGP < 9 (derived from the pin's major version).
    let agp_major: u32 = android_gradle_plugin
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or(0);
    let kotlin_android_plugin_line = if agp_major >= 9 {
        String::new()
    } else {
        format!("\n    kotlin(\"android\") version \"{kotlin_version}\"")
    };
    let junit_legacy = maven::JUNIT_LEGACY;
    let androidx_junit = maven::ANDROIDX_TEST_EXT_JUNIT;
    let espresso_core = maven::ANDROIDX_TEST_ESPRESSO_CORE;
    let ktlint_gradle_plugin = maven::KTLINT_GRADLE_PLUGIN;
    let ktlint_version = maven::KTLINT;
    let gradle_versions_plugin = maven::GRADLE_VERSIONS_PLUGIN;
    let kotlinx_coroutines = maven::KOTLINX_COROUTINES_CORE;
    let jackson = maven::JACKSON;
    let vanniktech_plugin = maven::VANNIKTECH_MAVEN_PUBLISH;
    let _ = toolchain::ANDROID_JVM_TARGET;

    let android_namespace = namespace(config);
    let compile_sdk_val = compile_sdk(config);
    let min_sdk_val = min_sdk(config);
    let android_jvm_target = jvm_target(config);
    let group_id = aar_group_id(config);
    let artifact_id = aar_artifact_id(config);
    let resolved_version = config.resolved_version().unwrap_or_else(|| "0.0.0".to_string());
    let version_placeholder = resolved_version.as_str();
    let jni_crate_path = config.jni_crate_path();
    let jni_lib_name = config.jni_lib_name();

    // Host-native capsule (Language) passthrough: depend on ktreesitter so the generated
    // facade can construct its `Language` from the native pointer. `package` is a Gradle
    // `group:artifact` coordinate (e.g. `io.github.tree-sitter:ktreesitter`).
    let capsule_deps: String = {
        let mut deps: Vec<(String, String)> = config
            .kotlin_android
            .as_ref()
            .map(|c| {
                c.capsule_types
                    .values()
                    .filter(|cap| !cap.package.is_empty())
                    .map(|cap| (cap.package.clone(), cap.package_version.clone()))
                    .collect()
            })
            .unwrap_or_default();
        deps.sort();
        deps.dedup();
        deps.iter()
            .map(|(coord, ver)| format!("\n    implementation(\"{coord}:{ver}\")"))
            .collect()
    };

    // Build pom metadata from config.scaffold
    let meta = scaffold_meta(config);

    // Derive SCM URLs from repository URL
    let repo_url = meta.repository.as_deref().unwrap_or_else(|| {
        panic!("Kotlin Android scaffold requires package metadata repository; set package_metadata.repository or scaffold.repository")
    });
    let repo_path = repo_url
        .strip_prefix("https://github.com/")
        .or_else(|| repo_url.strip_prefix("http://github.com/"))
        .unwrap_or(repo_url.trim_start_matches("https://"));

    // License URL mapping
    let license = meta.license.as_deref().unwrap_or_else(|| {
        panic!("Kotlin Android scaffold requires package metadata license; set package_metadata.license or scaffold.license")
    });
    let license_url = match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    };

    // Build licenses block
    let licenses_block = if license_url.is_empty() {
        format!(
            "licenses {{\n            license {{\n                name.set(\"{}\")\n            }}\n        }}",
            xml_escape(license)
        )
    } else {
        format!(
            "licenses {{\n            license {{\n                name.set(\"{}\")\n                url.set(\"{}\")\n            }}\n        }}",
            xml_escape(license),
            xml_escape(license_url)
        )
    };

    // Build developers block from authors (if any)
    let developers_block = if meta.authors.is_empty() {
        "\n".to_string() // Just newline if no developers
    } else {
        let devs: Vec<String> = meta
            .authors
            .iter()
            .map(|a| {
                let (name, email) = parse_author(a);
                format!(
                    "            developer {{\n                name.set(\"{}\")\n                email.set(\"{}\")\n            }}",
                    xml_escape(name),
                    xml_escape(email)
                )
            })
            .collect();
        format!("\n        developers {{\n{}\n        }}\n", devs.join("\n"))
    };

    format!(
        r#"// Generated by alef. Do not edit by hand.

import com.vanniktech.maven.publish.AndroidSingleVariantLibrary
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

buildscript {{
    dependencies {{
        classpath("com.vanniktech:gradle-maven-publish-plugin:{vanniktech_plugin}")
    }}
}}

plugins {{
    id("com.android.library") version "{android_gradle_plugin}"{kotlin_android_plugin_line}
    id("com.vanniktech.maven.publish") version "{vanniktech_plugin}"
    id("org.jlleitschuh.gradle.ktlint") version "{ktlint_gradle_plugin}"
    id("com.github.ben-manes.versions") version "{gradle_versions_plugin}"
}}

android {{
    namespace = "{android_namespace}"
    compileSdk = {compile_sdk_val}

    defaultConfig {{
        minSdk = {min_sdk_val}
        consumerProguardFiles("consumer-rules.pro")
    }}

    compileOptions {{
        sourceCompatibility = JavaVersion.VERSION_{android_jvm_target}
        targetCompatibility = JavaVersion.VERSION_{android_jvm_target}
    }}

    sourceSets {{
        getByName("main") {{
            jniLibs.srcDirs("src/main/jniLibs")
        }}
    }}
}}

kotlin {{
    compilerOptions {{
        jvmTarget.set(JvmTarget.JVM_{android_jvm_target})
    }}
}}

ktlint {{
    version.set("{ktlint_version}")
    android.set(true)
    ignoreFailures.set(false)
}}

dependencies {{
    implementation("org.jetbrains.kotlin:kotlin-stdlib")
    // Generated Kotlin facade uses suspend functions and Flow wrappers, both of
    // which require kotlinx-coroutines-android (transitively pulls -core).
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:{kotlinx_coroutines}")
    // Generated sealed-class DTOs use Jackson @JsonDeserialize for polymorphic
    // serde-tagged unions; jackson-module-kotlin is required for Kotlin
    // data-class deserialization (handles nullable, default values, etc.).
    // jackson-datatype-jdk8 is required because the generated DefaultClient.kt
    // registers Jdk8Module for Optional<T> / java.util.Optional support.
    implementation("com.fasterxml.jackson.core:jackson-databind:{jackson}")
    implementation("com.fasterxml.jackson.module:jackson-module-kotlin:{jackson}")
    implementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}"){capsule_deps}
    testImplementation("junit:junit:{junit_legacy}")
    androidTestImplementation("androidx.test.ext:junit:{androidx_junit}")
    androidTestImplementation("androidx.test.espresso:espresso-core:{espresso_core}")
}}

// Build host JNI library for JVM unit tests (macOS/Linux/Windows).
// The generated Kotlin Bridge object calls System.loadLibrary("{jni_lib_name}") for JVM
// unit tests running on developer machines. This task builds the host-platform binary
// and stages it into src/test/resources/host-jni/<platform>/ for the test loader.
// Set alef.skipHostJni=true to disable this (e.g., in publish-only builds).
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

        val hostPlatform = when {{
            System.getProperty("os.name").lowercase().contains("mac") -> "darwin"
            System.getProperty("os.name").lowercase().contains("win") -> "windows"
            else -> "linux"
        }}
        val jniCratePath = file("{jni_crate_path}")
        val buildDir = jniCratePath.resolve("target/release")

        // Map host platform to library filename
        val libName = when (hostPlatform) {{
            "darwin" -> "lib{jni_lib_name}.dylib"
            "windows" -> "{jni_lib_name}.dll"
            else -> "lib{jni_lib_name}.so"  // linux
        }}

        from(buildDir) {{
            include(libName)
        }}
        into(layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform"))
    }}
}}

tasks.withType<Test> {{
    if (project.properties["alef.skipHostJni"] != "true") {{
        val hostPlatform = when {{
            System.getProperty("os.name").lowercase().contains("mac") -> "darwin"
            System.getProperty("os.name").lowercase().contains("win") -> "windows"
            else -> "linux"
        }}
        systemProperty(
            "java.library.path",
            project.layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform").asFile.absolutePath
        )
        dependsOn("copyHostJni")
    }}
}}

// `processDebugUnitTestJavaRes` and `processReleaseUnitTestJavaRes` package the
// `src/test/resources` tree into the unit-test runtime classpath. They consume
// the dylib emitted by `copyHostJni`, so AGP 8.10+ requires an explicit
// dependency declaration to satisfy Gradle's task-output validation.
tasks.matching {{ it.name.startsWith("processDebug") || it.name.startsWith("processRelease") }}.configureEach {{
    if (project.properties["alef.skipHostJni"] != "true" && name.contains("UnitTestJavaRes")) {{
        dependsOn("copyHostJni")
    }}
}}

// Guard: fail the build if assembleRelease runs without jniLibs staged.
// This prevents accidental publication of jni-less AARs when gradle rebuilds
// during the publish phase. The publish workflow must extract jniLibs from
// pre-built AARs and stage them into src/main/jniLibs before invoking gradle.
// This check catches the bug where jniLibs are lost during publish-time rebuild.
tasks.register("validateJniLibsForRelease") {{
    doFirst {{
        if (gradle.taskGraph.hasTask("assembleRelease") || gradle.taskGraph.hasTask("publishAndReleaseToMavenCentral")) {{
            val jniLibsDir = file("src/main/jniLibs")
            if (!jniLibsDir.exists() || jniLibsDir.listFiles()?.isEmpty() != false) {{
                throw GradleException(
                    "FATAL: jniLibs directory is empty or missing. " +
                    "The Android AAR must include native .so libraries for ARM64 and x86_64. " +
                    "Ensure the publish workflow stages jniLibs from pre-built AARs " +
                    "into src/main/jniLibs/{{arm64-v8a,x86_64}}/ before invoking assembleRelease. " +
                    "Aborting to prevent shipping a jni-less AAR to Maven Central."
                )
            }}
        }}
    }}
}}

// Make assemble and publish tasks depend on the validation.
tasks.named("preBuild") {{
    dependsOn("validateJniLibsForRelease")
}}

mavenPublishing {{
    configure(AndroidSingleVariantLibrary(
        variant = "release",
        sourcesJar = com.vanniktech.maven.publish.SourcesJar.Sources(),
        javadocJar = com.vanniktech.maven.publish.JavadocJar.Empty(),
    ))

    publishToMavenCentral()
    signAllPublications()

    coordinates(
        groupId = "{group_id}",
        artifactId = "{artifact_id}",
        version = "{version_placeholder}",
    )

    pom {{
        name.set("{artifact_id}")
        description.set("{}")
        url.set("{}")
        {licenses_block}{developers_block}
        scm {{
            url.set("{}")
            connection.set("scm:git:git://github.com/{}.git")
            developerConnection.set("scm:git:ssh://git@github.com:{}.git")
        }}
    }}
}}
"#,
        xml_escape(&meta.description), // description.set({})
        xml_escape(repo_url),          // url.set({})
        xml_escape(repo_url),          // url.set({}) in scm block
        repo_path,                     // connection.set("scm:git:git://github.com/{}.git")
        repo_path,                     // developerConnection.set("scm:git:ssh://git@github.com:{}.git")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_gradle_includes_host_jni_tasks() {
        // Create a minimal ResolvedCrateConfig for testing.
        use crate::core::config::new_config::NewAlefConfig;

        let toml_str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example"

[crates.jni]

[crates.scaffold]
repository = "https://github.com/example/test-lib"
license = "MIT"
description = "Test library"
"#;

        let cfg: NewAlefConfig = toml::from_str(toml_str).unwrap();
        let resolved = cfg.resolve().unwrap();
        let config = &resolved[0];

        let gradle = emit(config);

        // Verify the emitted Gradle script contains the host JNI build task.
        assert!(
            gradle.contains(r#"tasks.register("buildHostJni", Exec::class)"#),
            "Gradle should contain buildHostJni task registration"
        );

        // Verify the copyHostJni task is present.
        assert!(
            gradle.contains(r#"tasks.register("copyHostJni", Copy::class)"#),
            "Gradle should contain copyHostJni task registration"
        );

        // Verify the Test task configuration is present.
        assert!(
            gradle.contains("tasks.withType<Test>"),
            "Gradle should configure tasks.withType<Test>"
        );

        // Verify the java.library.path property is set for tests.
        assert!(
            gradle.contains("java.library.path"),
            "Gradle should set java.library.path system property"
        );

        // Verify the opt-out property is documented.
        assert!(
            gradle.contains("alef.skipHostJni"),
            "Gradle should mention alef.skipHostJni opt-out"
        );
    }
}
