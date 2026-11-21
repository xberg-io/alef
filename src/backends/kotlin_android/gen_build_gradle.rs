//! `build.gradle.kts` emitter — full Android library project with
//! vanniktech maven-publish, ktlint hooks, bundled-Java-facade source set wiring,
//! and optional host JNI binary build for JVM unit tests.

use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions::{maven, toolchain};
use std::path::PathBuf;

use crate::backends::kotlin_android::naming::{
    aar_artifact_id, aar_group_id, abis, compile_sdk, jvm_target, min_sdk, namespace,
};
use crate::backends::kotlin_android::template_env::render;
use crate::scaffold::{parse_author, scaffold_meta, xml_escape};

mod author_metadata;

/// Workspace-root-relative notice files configured for the AAR, in declaration order.
fn bundled_notice_paths(config: &ResolvedCrateConfig) -> Vec<&str> {
    config
        .kotlin_android
        .as_ref()
        .map(|android| android.bundled_notices.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

/// Directory `[crates.kotlin_android] bundled_notices` paths are resolved against: the
/// configured workspace root, else the directory alef runs in (the workspace root for a
/// normal `alef generate`).
fn bundled_notices_root(config: &ResolvedCrateConfig) -> PathBuf {
    config
        .workspace_root
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Escape a value for a Kotlin string literal.
fn kotlin_string_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$")
}

/// Render the Gradle block that copies `bundled_notices` into the AAR.
///
/// Fails, naming every missing path, when a configured notice file does not exist under the
/// workspace root: silently emitting a copy task that fails at build time (or ships an AAR
/// without its notices) defeats the point of configuring them.
fn render_bundled_notices_block(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let paths = bundled_notice_paths(config);
    if paths.is_empty() {
        return Ok(String::new());
    }

    let root = bundled_notices_root(config);
    let mut missing = Vec::new();
    for path in &paths {
        let resolved = root.join(path);
        if !resolved.is_file() {
            missing.push(format!("`{path}` (resolved to {})", resolved.display()));
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "kotlin_android bundled_notices path(s) not found under workspace root `{}`: {}; \
             `[crates.kotlin_android] bundled_notices` paths are relative to the workspace root",
            root.display(),
            missing.join(", ")
        );
    }

    let literals = paths
        .iter()
        .map(|path| format!("    \"{}\",", kotlin_string_literal(path)))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"// Bundle the configured third-party notice files into the AAR. The AAR packages
// src/main/resources, so each workspace-root-relative bundled_notices path lands at
// META-INF/<path>; preBuild depends on the copy, so every variant (AGP 8 and AGP 9 alike)
// packages the files. The path list is emitted in config order, keeping the file stable.
val bundledNoticePaths = listOf(
{literals}
)
tasks.register("bundleNotices", Copy::class) {{
    val configuredRoot = project.providers.gradleProperty("alef.workspaceRoot").orNull
    val workspaceRoot = configuredRoot?.let(::file)
        ?: generateSequence(projectDir) {{ it.parentFile }}
            .firstOrNull {{ candidate -> bundledNoticePaths.all {{ candidate.resolve(it).isFile }} }}
        ?: throw GradleException(
            "Cannot locate a workspace root containing every bundled_notices path (" +
                bundledNoticePaths.joinToString(", ") + ") from $projectDir; " +
                "set -Palef.workspaceRoot=/absolute/path/to/workspace"
        )
    from(workspaceRoot) {{
        include(bundledNoticePaths)
    }}
    into(layout.projectDirectory.dir("src/main/resources/META-INF"))
    description = "Bundle third-party notice files into the AAR"
}}

tasks.named("preBuild") {{
    dependsOn("bundleNotices")
}}
"#
    ))
}

/// Render the Gradle task that cross-compiles the JNI crate into `src/main/jniLibs/<abi>/`.
///
/// The ABI list is the same [`abis`] the `.gitkeep` scaffolder uses, so the directories alef
/// creates and the directories alef fills can never name two different sets.
fn render_jni_libs_task(config: &ResolvedCrateConfig, jni_manifest_workspace_path: &str, jni_lib_name: &str) -> String {
    let abi_literals = abis(config)
        .iter()
        .map(|abi| format!("\"{abi}\""))
        .collect::<Vec<_>>()
        .join(", ");
    render(
        "android_jni_libs_task.jinja",
        crate::alef_context! {
            abi_literals => abi_literals,
            jni_lib_name => jni_lib_name,
            jni_manifest_workspace_path => jni_manifest_workspace_path,
        },
    )
}

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
pub fn emit(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let kotlin_version = maven::KOTLIN_JVM_PLUGIN;
    let android_gradle_plugin = maven::ANDROID_GRADLE_PLUGIN;
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
    let jni_manifest_workspace_path = format!("crates/{}-jni/Cargo.toml", config.jni_crate_base());
    let jni_lib_name = config.jni_lib_name();
    let android_jni_libs_task = render_jni_libs_task(config, &jni_manifest_workspace_path, &jni_lib_name);

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
            .map(|(coord, ver)| format!("\n    api(\"{coord}:{ver}\")"))
            .collect()
    };

    let meta = scaffold_meta(config);

    // `build.gradle.kts` is regenerated on every `alef generate`, unlike the
    // sibling `java`/`kotlin`/`r`/`gleam` scaffolders (which bail once, at
    // `alef scaffold` time, when publish metadata is missing). Panicking here
    // aborted every plain `generate` for a crate that hadn't configured Maven
    // Central metadata yet. Repository and license only feed the POM's
    // publishing block, so — matching the "must not invent repository
    // metadata" convention the C#, WASM and npm scaffolders already follow —
    // generation now degrades gracefully: the corresponding POM section is
    // omitted and a warning names the missing config. ~keep
    if meta.repository.is_none() {
        tracing::warn!(
            crate_name = %config.name,
            "kotlin_android build.gradle.kts has no package metadata repository configured; \
             set package_metadata.repository or scaffold.repository to populate the published \
             POM's project URL and SCM block. Generating without them."
        );
    }
    if meta.license.is_none() {
        tracing::warn!(
            crate_name = %config.name,
            "kotlin_android build.gradle.kts has no package metadata license configured; \
             set package_metadata.license or scaffold.license to populate the published POM's \
             license block. Generating without it."
        );
    }

    let repo_url = meta.repository.as_deref();
    let repo_path = repo_url.map(|repo_url| {
        repo_url
            .strip_prefix("https://github.com/")
            .or_else(|| repo_url.strip_prefix("http://github.com/"))
            .unwrap_or(repo_url.trim_start_matches("https://"))
            .to_string()
    });

    let license = meta.license.as_deref();
    let license_url = license.map(|license| match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    });

    let licenses_block = match (license, license_url) {
        (Some(license), Some(url)) if !url.is_empty() => format!(
            "licenses {{\n            license {{\n                name.set(\"{}\")\n                url.set(\"{}\")\n            }}\n        }}",
            xml_escape(license),
            xml_escape(url)
        ),
        (Some(license), _) => format!(
            "licenses {{\n            license {{\n                name.set(\"{}\")\n            }}\n        }}",
            xml_escape(license)
        ),
        (None, _) => String::new(),
    };

    let project_url_line = repo_url
        .map(|url| format!("url.set(\"{}\")\n        ", xml_escape(url)))
        .unwrap_or_default();

    let scm_block = match (repo_url, repo_path.as_deref()) {
        (Some(url), Some(path)) => format!(
            "\n        scm {{\n            url.set(\"{}\")\n            connection.set(\"scm:git:git://github.com/{}.git\")\n            developerConnection.set(\"scm:git:ssh://git@github.com:{}.git\")\n        }}",
            xml_escape(url),
            path,
            path
        ),
        _ => String::new(),
    };

    let developers_block = author_metadata::render_developers(&meta.authors);

    let marker = crate::core::hash::SELF_MARKING_HEADER_LINE;
    let bundled_notices_block = render_bundled_notices_block(config)?;
    Ok(format!(
        r#"{marker}

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
    if (project.providers.gradleProperty("alef.skipHostJni").orNull != "true") {{
        val configuredManifest = project.findProperty("alef.jniManifestPath") as String?
        val jniManifest = configuredManifest?.let(::file) ?: generateSequence(projectDir) {{ it.parentFile }}
            .map {{ it.resolve("{jni_manifest_workspace_path}") }}
            .firstOrNull {{ it.isFile }}
            ?: throw GradleException(
                "Cannot locate {jni_manifest_workspace_path} from $projectDir; " +
                    "set -Palef.jniManifestPath=/absolute/path/to/Cargo.toml"
            )
        description = "Build host-platform JNI library from $jniManifest"
        commandLine("cargo", "build", "--release", "--manifest-path", jniManifest.absolutePath)
        errorOutput = System.err
    }} else {{
        description = "Build host JNI (disabled via alef.skipHostJni=true)"
        commandLine("true")
    }}
}}

tasks.register("copyHostJni", Copy::class) {{
    if (project.providers.gradleProperty("alef.skipHostJni").orNull != "true") {{
        description = "Copy host JNI library to test resources"
        dependsOn("buildHostJni")

        val hostPlatform = when {{
            System.getProperty("os.name").lowercase().contains("mac") -> "darwin"
            System.getProperty("os.name").lowercase().contains("win") -> "windows"
            else -> "linux"
        }}
        val configuredManifest = project.findProperty("alef.jniManifestPath") as String?
        val jniManifest = configuredManifest?.let(::file) ?: generateSequence(projectDir) {{ it.parentFile }}
            .map {{ it.resolve("{jni_manifest_workspace_path}") }}
            .firstOrNull {{ it.isFile }}
            ?: throw GradleException("Cannot locate {jni_manifest_workspace_path} from $projectDir")
        val workspaceRoot = jniManifest.parentFile.parentFile.parentFile
        val buildDir = (project.findProperty("native.lib.path") as String?)?.let(::file)
            ?: workspaceRoot.resolve("target/release")

        // Map host platform to library filename
        val libName = when (hostPlatform) {{
            "darwin" -> "lib{jni_lib_name}.dylib"
            "windows" -> "{jni_lib_name}.dll"
            else -> "lib{jni_lib_name}.so" // linux
        }}

        from(buildDir) {{
            include(libName)
        }}
        into(layout.projectDirectory.dir("src/test/resources/host-jni/$hostPlatform"))
    }}
}}

tasks.withType<Test> {{
    if (project.providers.gradleProperty("alef.skipHostJni").orNull != "true") {{
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

// `process*UnitTestJavaRes` and AGP 9's `compress*UnitTestJavaRes` package the
// `src/test/resources` tree into the unit-test runtime classpath. They consume
// the dylib emitted by `copyHostJni`, so AGP 8.10+ requires an explicit
// dependency declaration to satisfy Gradle's task-output validation.
tasks.matching {{ it.name.endsWith("UnitTestJavaRes") && (it.name.startsWith("process") || it.name.startsWith("compress")) }}.configureEach {{
    if (project.providers.gradleProperty("alef.skipHostJni").orNull != "true") {{
        dependsOn("copyHostJni")
    }}
}}

// Guard: fail the build if assembleRelease runs without jniLibs staged.
// This prevents accidental publication of jni-less AARs when gradle rebuilds
// during the publish phase. The publish workflow must extract jniLibs from
// pre-built AARs and stage them into src/main/jniLibs before invoking gradle.
// This check catches the bug where jniLibs are lost during publish-time rebuild.
// NOTE: gate on task *name* via allTasks.any, not gradle.taskGraph.hasTask(name),
// because hasTask(String) matches the fully-qualified path (":assembleRelease")
// and a bare name never matches — which silently disabled this guard and let a
// jni-less AAR ship.
tasks.register("validateJniLibsForRelease") {{
    doFirst {{
        val releaseAssemble = gradle.taskGraph.allTasks.any {{
            it.name == "assembleRelease" || it.name == "publishAndReleaseToMavenCentral"
        }}
        if (releaseAssemble) {{
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
            // Every ABI directory must also contain the JNI crate's own library.
            // A non-empty jniLibs holding a differently-named .so (e.g. the C-FFI
            // lib*_ffi.so instead of lib{jni_lib_name}.so) passes the emptiness
            // check above but fails at runtime with UnsatisfiedLinkError, because
            // System.loadLibrary("{jni_lib_name}") needs the JNI entry points.
            val expectedJniLib = "lib{jni_lib_name}.so"
            val abisMissingJniLib = (jniLibsDir.listFiles()?.filter {{ it.isDirectory }} ?: emptyList())
                .filter {{ abiDir -> !abiDir.resolve(expectedJniLib).exists() }}
            if (abisMissingJniLib.isNotEmpty()) {{
                throw GradleException(
                    "FATAL: " + expectedJniLib + " is missing from jniLibs ABI dir(s): " +
                    abisMissingJniLib.joinToString(", ") {{ it.name }} + ". " +
                    "A differently-named native library (e.g. the C-FFI lib*_ffi.so) does not " +
                    "export the JNI symbols System.loadLibrary(\"{jni_lib_name}\") requires and " +
                    "fails at runtime. Aborting to prevent shipping a broken AAR to Maven Central."
                )
            }}
        }}
    }}
}}

// Make assemble and publish tasks depend on the validation.
tasks.named("preBuild") {{
    dependsOn("validateJniLibsForRelease")
}}

{android_jni_libs_task}
{bundled_notices_block}mavenPublishing {{
    configure(
        AndroidSingleVariantLibrary(
            variant = "release",
            sourcesJar = com.vanniktech.maven.publish.SourcesJar.Sources(),
            javadocJar = com.vanniktech.maven.publish.JavadocJar.Empty(),
        ),
    )

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
        {project_url_line}{licenses_block}{developers_block}{scm_block}
    }}
}}
"#,
        xml_escape(&meta.description),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_gradle_includes_host_jni_tasks() {
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

        let gradle = emit(config).expect("fixture config emits build.gradle.kts");

        assert!(
            gradle.contains(r#"tasks.register("buildHostJni", Exec::class)"#),
            "Gradle should contain buildHostJni task registration"
        );

        assert!(
            gradle.contains(r#"tasks.register("copyHostJni", Copy::class)"#),
            "Gradle should contain copyHostJni task registration"
        );

        assert!(
            gradle.contains("tasks.withType<Test>"),
            "Gradle should configure tasks.withType<Test>"
        );

        assert!(
            gradle.contains("java.library.path"),
            "Gradle should set java.library.path system property"
        );

        assert!(
            gradle.contains("alef.skipHostJni"),
            "Gradle should mention alef.skipHostJni opt-out"
        );
        assert!(
            gradle.contains(r#"generateSequence(projectDir) { it.parentFile }"#)
                && gradle.contains(r#"it.resolve("crates/test-lib-jni/Cargo.toml")"#),
            "Gradle should locate the JNI manifest independently of configured output depth"
        );
        assert!(
            gradle.contains(r#"project.findProperty("alef.jniManifestPath")"#),
            "Gradle should allow an explicit JNI manifest override"
        );
        assert!(
            gradle.contains(r#"workspaceRoot.resolve("target/release")"#)
                && gradle.contains(r#"project.findProperty("native.lib.path")"#),
            "Gradle should read host artifacts from the Cargo workspace target directory"
        );
        assert!(
            !gradle.contains(r#"jniCratePath.resolve("target/release")"#),
            "Gradle must not assume a per-crate Cargo target directory"
        );
    }

    #[test]
    fn build_gradle_is_ktlint_clean_for_known_violations() {
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
        let gradle = emit(&resolved[0]).expect("fixture config emits build.gradle.kts");

        // ktlint "Unnecessary long whitespace": exactly one space before a trailing comment.
        assert!(
            !gradle.contains(".so\"  // linux"),
            "ktlint rejects the double space before the trailing `// linux` comment"
        );
        assert!(
            gradle.contains(".so\" // linux"),
            "the else branch must keep a single-spaced trailing `// linux` comment"
        );

        // ktlint "Missing newline after (" / "before )": the outer configure(...) call wrapping ~keep
        // the multi-line AndroidSingleVariantLibrary(...) argument must break after `configure(`. ~keep
        assert!(
            gradle.contains("configure(\n        AndroidSingleVariantLibrary("),
            "configure(...) must wrap its multiline argument onto its own line for ktlint"
        );
        assert!(
            !gradle.contains("configure(AndroidSingleVariantLibrary("),
            "configure(AndroidSingleVariantLibrary( on one line trips ktlint's wrapping rule"
        );
    }

    #[test]
    fn build_gradle_validates_jni_lib_present_per_abi() {
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
        let jni_lib_name = config.jni_lib_name();

        let gradle = emit(config).expect("fixture config emits build.gradle.kts");

        // The release guard must assert the correctly-named JNI library is present in ~keep
        // each ABI dir, not merely that jniLibs is non-empty (which a stray C-FFI ~keep
        // lib*_ffi.so would satisfy while failing at runtime with UnsatisfiedLinkError). ~keep
        assert!(
            gradle.contains(&format!("val expectedJniLib = \"lib{jni_lib_name}.so\"")),
            "guard must check for the JNI-named library per ABI"
        );
        assert!(
            gradle.contains("abisMissingJniLib"),
            "guard must collect ABI dirs missing the JNI library"
        );
        assert!(
            gradle.contains("is missing from jniLibs ABI dir(s)"),
            "guard must fail with an actionable message naming the missing ABI dirs"
        );
        assert!(
            gradle.contains("gradle.taskGraph.allTasks.any"),
            "guard must gate on task name via allTasks.any (hasTask(String) matches the \
             qualified path and silently disabled the guard)"
        );
        assert!(
            !gradle.contains("gradle.taskGraph.hasTask(\"assembleRelease\")"),
            "guard must not use hasTask(String) (bare name never matches the qualified task path)"
        );
    }

    /// Shared neutral fixture for the Android ABI cross-compile contract. `extra` is appended
    /// verbatim so a test can add `[crates.kotlin_android] abis = [...]`.
    fn android_config_toml(extra: &str) -> String {
        format!(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example"
{extra}

[crates.jni]

[crates.scaffold]
repository = "https://github.com/example/test-lib"
license = "MIT"
description = "Test library"
"#
        )
    }

    fn emit_android_gradle(extra: &str) -> String {
        use crate::core::config::new_config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(&android_config_toml(extra)).expect("fixture config parses");
        let resolved = cfg.resolve().expect("fixture config resolves");
        emit(&resolved[0]).expect("fixture config emits build.gradle.kts")
    }

    #[test]
    fn build_gradle_cross_compiles_the_jni_crate_for_every_configured_abi() {
        let gradle = emit_android_gradle("");

        assert!(
            gradle.contains(r#"tasks.register("buildAndroidJniLibs", Exec::class)"#),
            "the release guard demands lib*_jni.so per ABI, so something must produce it"
        );
        assert!(
            gradle.contains(r#"val androidAbis = listOf("arm64-v8a", "x86_64")"#),
            "the cross-compile must target the configured ABI list, defaulted to arm64-v8a/x86_64"
        );
        assert!(
            gradle.contains(r#""cargo", "ndk""#),
            "a host `cargo build` cannot produce an Android ABI library; cargo-ndk must be invoked"
        );
        assert!(
            gradle.contains(r#"listOf("-t", abi)"#),
            "each configured ABI must be passed to cargo-ndk as its own -t target"
        );
        assert!(
            gradle.contains(r#""-o","#) && gradle.contains("jniLibsRoot.absolutePath"),
            "cargo-ndk must write straight into src/main/jniLibs so the AAR picks the libraries up"
        );
        assert!(
            gradle.contains(r#"val jniLibsRoot = layout.projectDirectory.dir("src/main/jniLibs").asFile"#),
            "the cross-compile output root must be the jniLibs source dir the AAR packages"
        );
        assert!(
            gradle.contains(r#"it.resolve("crates/test-lib-jni/Cargo.toml")"#),
            "the cross-compile must build the crate's own JNI manifest, derived from config"
        );
    }

    #[test]
    fn android_cross_compile_is_wired_ahead_of_the_release_guard_and_the_jni_libs_merge() {
        let gradle = emit_android_gradle("");

        assert!(
            gradle.contains("tasks.named(\"validateJniLibsForRelease\") {\n    dependsOn(\"buildAndroidJniLibs\")"),
            "the guard must depend on the task that satisfies it, or the guard still fails first"
        );
        assert!(
            gradle.contains(r#"it.name.endsWith("JniLibFolders")"#),
            "AGP's jniLibs merge consumes the cross-compiled output and needs an explicit dependency"
        );
    }

    #[test]
    fn android_cross_compile_skips_debug_graphs_prestaged_libs_and_explicit_opt_out() {
        let gradle = emit_android_gradle("");

        assert!(
            gradle.contains("alef.skipAndroidJni"),
            "a publish workflow that stages prebuilt libraries needs an unconditional opt-out"
        );
        assert!(
            gradle.contains(r#"androidAbis.any { abi -> !jniLibsRoot.resolve(abi).resolve(expectedJniLib).isFile }"#),
            "already-staged ABI libraries must not be rebuilt, so an NDK-less publish runner keeps working"
        );
        assert!(
            gradle.contains("onlyIf {"),
            "the release-graph, prestaged and opt-out conditions must gate execution, not configuration"
        );
        assert!(
            gradle.contains("cargo install cargo-ndk"),
            "a missing cross-compiler must name its own remediation instead of `command not found`"
        );
    }

    #[test]
    fn android_cross_compile_honors_a_configured_abi_list() {
        let gradle = emit_android_gradle(r#"abis = ["armeabi-v7a"]"#);

        assert!(
            gradle.contains(r#"val androidAbis = listOf("armeabi-v7a")"#),
            "the ABI list must come from `[crates.kotlin_android] abis`, never a hardcoded pair"
        );
        assert!(
            !gradle.contains("arm64-v8a\", \"x86_64"),
            "a configured ABI list must replace the default, not be appended to it"
        );
    }

    /// Regression: current Gradle ("The Project.getProperties method has been deprecated.
    /// This will fail with an error in Gradle 10.") rejects `project.properties[...]` reads.
    /// Every alef-owned opt-out flag must go through the lazy `providers.gradleProperty(...)`
    /// accessor instead, both in this file's own host-JNI tasks and in the
    /// `android_jni_libs_task.jinja` cross-compile task it embeds.
    #[test]
    fn gradle_property_reads_use_the_provider_api_not_the_deprecated_properties_map() {
        let gradle = emit_android_gradle("");

        assert!(
            gradle.contains(r#"project.providers.gradleProperty("alef.skipHostJni").orNull != "true""#),
            "buildHostJni-family tasks must read alef.skipHostJni via providers.gradleProperty: {gradle}"
        );
        assert!(
            gradle.contains(r#"project.providers.gradleProperty("alef.skipAndroidJni").orNull != "true""#),
            "the Android cross-compile task must read alef.skipAndroidJni via providers.gradleProperty: {gradle}"
        );
        assert!(
            !gradle.contains("project.properties["),
            "no task may read a Gradle property via the deprecated project.properties map: {gradle}"
        );
    }

    /// Regression: current AGP ("Setting the namespace via the package attribute is no
    /// longer supported") requires the namespace to live in `android { namespace = ... }`
    /// rather than the manifest's `package` attribute (see `gen_manifest` tests for that half).
    #[test]
    fn android_block_sets_the_namespace_that_the_manifest_no_longer_carries() {
        let gradle = emit_android_gradle("");

        assert!(
            gradle.contains("namespace = \"dev.example\""),
            "the android {{ }} block must set namespace from the resolved kotlin_android package: {gradle}"
        );
    }

    fn resolve_android_config(extra: &str) -> crate::core::config::ResolvedCrateConfig {
        use crate::core::config::new_config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(&android_config_toml(extra)).expect("fixture config parses");
        cfg.resolve().expect("fixture config resolves").remove(0)
    }

    /// The AAR carries every configured notice file at `META-INF/<path>`; the copier must name
    /// exactly the configured paths, in config order, and hang off `preBuild` so every variant
    /// (release assembly, AAR packaging) sees them. AGP 8 and AGP 9 both wire `preBuild` ahead
    /// of resource processing, so the same block covers both.
    #[test]
    fn bundled_notice_files_are_copied_into_the_aar_meta_inf() {
        let workspace = tempfile::tempdir().expect("tempdir");
        std::fs::write(workspace.path().join("LICENSE"), b"license text").expect("write LICENSE");
        std::fs::create_dir_all(workspace.path().join("legal")).expect("create legal dir");
        std::fs::write(workspace.path().join("legal/THIRD_PARTY_LICENSES.md"), b"notices").expect("write notices");

        let mut config = resolve_android_config(r#"bundled_notices = ["LICENSE", "legal/THIRD_PARTY_LICENSES.md"]"#);
        config.workspace_root = Some(workspace.path().to_path_buf());

        let gradle = emit(&config).expect("configured notice files exist");

        assert!(
            gradle.contains(
                "val bundledNoticePaths = listOf(\n    \"LICENSE\",\n    \"legal/THIRD_PARTY_LICENSES.md\",\n)"
            ),
            "the copier must name the configured workspace-relative paths verbatim, in order:\n{gradle}"
        );
        assert!(
            gradle.contains(r#"tasks.register("bundleNotices", Copy::class)"#),
            "the notice copier must be a Copy task:\n{gradle}"
        );
        assert!(
            gradle.contains(r#"into(layout.projectDirectory.dir("src/main/resources/META-INF"))"#),
            "the copy target must be the resource tree the AAR packages:\n{gradle}"
        );
        assert!(
            gradle.contains("tasks.named(\"preBuild\") {\n    dependsOn(\"bundleNotices\")\n}"),
            "preBuild must depend on the copier, or variants package resources before it runs:\n{gradle}"
        );
        assert_eq!(
            gradle,
            emit(&config).expect("second emission succeeds"),
            "repeated generation must be byte-stable"
        );
    }

    #[test]
    fn unconfigured_bundled_notices_emit_no_notice_block() {
        let gradle = emit_android_gradle("");

        assert!(
            !gradle.contains("bundleNotices") && !gradle.contains("META-INF"),
            "an empty bundled_notices list must not emit a copier or its wiring:\n{gradle}"
        );
    }

    #[test]
    fn a_missing_bundled_notice_fails_generation_naming_the_path() {
        let workspace = tempfile::tempdir().expect("tempdir");
        std::fs::write(workspace.path().join("LICENSE"), b"license text").expect("write LICENSE");

        let mut config = resolve_android_config(r#"bundled_notices = ["LICENSE", "THIRD_PARTY_LICENSES.md"]"#);
        config.workspace_root = Some(workspace.path().to_path_buf());

        let error = emit(&config).expect_err("a configured notice file that does not exist must fail generation");
        let message = error.to_string();

        assert!(
            message.contains("THIRD_PARTY_LICENSES.md"),
            "the failure must name the missing path; got: {message}"
        );
        assert!(
            message.contains(&workspace.path().display().to_string()),
            "the failure must name the workspace root that was searched; got: {message}"
        );
        assert!(
            !message.contains("`LICENSE`"),
            "only the missing path should be reported; got: {message}"
        );
    }

    /// ben-manes/gradle-versions-plugin releases through 0.54.0 predate the Gradle 9 rework
    /// (v0.55.0's per-project aggregation, which also raised the plugin's floor to Gradle 8.4);
    /// running `dependencyUpdates` from one of those releases against alef's Gradle 9 wrapper is
    /// exactly the failure four of five consumer repos papered over with a no-op override. This
    /// parses the version the scaffold actually emits and checks it numerically against the 0.55
    /// boundary, so a well-meaning revert of the `GRADLE_VERSIONS_PLUGIN` pin back toward "0.54.0"
    /// fails this test instead of silently reintroducing the incompatibility. ~keep
    #[test]
    fn build_gradle_pins_a_gradle_9_compatible_versions_plugin() {
        let gradle = emit_android_gradle("");

        let version = gradle
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix(r#"id("com.github.ben-manes.versions") version ""#)
            })
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("expected a ben-manes versions plugin line, got: {gradle}"));

        let mut parts = version.split('.');
        let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);

        assert!(
            (major, minor) >= (0, 55),
            "gradle-versions-plugin {version} predates the Gradle 9 fix in v0.55.0; \
             bump `template_versions::maven::GRADLE_VERSIONS_PLUGIN`"
        );
    }
}

#[cfg(test)]
mod resource_dependency_tests {
    #[test]
    #[ignore = "requires managed Gradle and JDK; run the explicit native task-graph gate"]
    fn package_compress_resource_dependencies() {
        let config: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android"]
[[crates]]
name = "resource-probe"
sources = []
[crates.kotlin_android]
package = "dev.example"
[crates.jni]
"#,
        )
        .unwrap();
        let resolved = config.resolve().unwrap();
        let generated = super::emit(&resolved[0]).expect("fixture config emits build.gradle.kts");
        crate::test_support::kotlin_gradle::assert_resource_edges(&generated);
    }
}
