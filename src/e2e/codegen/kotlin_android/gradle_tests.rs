//! Regression tests for `gradle.rs`'s build.gradle.kts / settings.gradle.kts renderers.
//!
//! Split out of `gradle.rs` itself once that file passed the 1,000-line
//! file-modularization cap (CLAUDE.md `file-modularization`) -- purely a move, no
//! behavior change.

#[cfg(test)]
mod tests {
    use super::super::gradle::{
        KotlinAndroidBuildGradleInputs, render_build_gradle_kotlin_android, render_settings_gradle_kotlin_android,
    };

    /// Regression: the kotlin-android build.gradle.kts must declare
    /// `jackson-module-kotlin` so that Jackson can deserialize Kotlin data
    /// classes (which have no default constructor).  Without it, any test that
    /// calls `MAPPER.readValue(...)` against a Kotlin data class throws
    /// `InvalidDefinitionException: No suitable constructor found`.
    #[test]
    fn build_gradle_kotlin_android_includes_jackson_module_kotlin() {
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate.samplellm.android",
            maven_coordinate: "dev.sample_crate:demo-client-android:1.0.0",
            dep_mode: crate::e2e::config::DependencyMode::Local,
            jni_lib_name: "demo_client_jni",
            jni_crate_path: "../../crates/demo-client-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });
        assert!(
            output.contains("jackson-module-kotlin"),
            "build.gradle.kts must depend on jackson-module-kotlin, got:\n{output}"
        );
    }

    /// Regression: build.gradle.kts must pin jackson-annotations to
    /// `JACKSON_ANNOTATIONS`, not the `JACKSON_E2E` version used for
    /// jackson-databind/jackson-datatype-jdk8/jackson-module-kotlin.
    /// jackson-annotations stopped publishing patch-version releases after 2.19.x
    /// (2.20, 2.21, 2.22, ... with no third component); reusing `JACKSON_E2E`'s
    /// `major.minor.patch` value resolves a coordinate that was never published on
    /// Maven Central, so `:generateDebugUnitTestStubRFile` (and every other task
    /// needing the test classpath resolved) fails before Kotlin compilation starts.
    #[test]
    fn build_gradle_kotlin_android_pins_jackson_annotations_to_its_own_version_scheme() {
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate.samplellm.android",
            maven_coordinate: "dev.sample_crate:demo-client-android:1.0.0",
            dep_mode: crate::e2e::config::DependencyMode::Local,
            jni_lib_name: "demo_client_jni",
            jni_crate_path: "../../crates/demo-client-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });
        let annotations_line = output
            .lines()
            .find(|line| line.contains("jackson-annotations"))
            .expect("jackson-annotations dependency line must be emitted");
        assert!(
            annotations_line.contains(&format!(
                "jackson-annotations:{}\"",
                crate::core::template_versions::maven::JACKSON_ANNOTATIONS
            )),
            "jackson-annotations must use JACKSON_ANNOTATIONS's version scheme, got:\n{annotations_line}"
        );
        assert!(
            !annotations_line.contains(crate::core::template_versions::maven::JACKSON_E2E),
            "jackson-annotations must not reuse JACKSON_E2E's patch-versioned scheme, got:\n{annotations_line}"
        );
    }

    /// Regression: `[crates.e2e.env]` vars must be forwarded into the Gradle test
    /// worker's process environment via `environment(...)` so the JNI-loaded native
    /// library reads them through libc `getenv`. Java cannot mutate `environ`
    /// in-process, so it must be set at worker-fork time.
    #[test]
    fn build_gradle_kotlin_android_forwards_e2e_env_to_test_worker() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("MY_SERVICE_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        for dep_mode in [
            crate::e2e::config::DependencyMode::Registry,
            crate::e2e::config::DependencyMode::Local,
        ] {
            let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
                kotlin_pkg_id: "dev.sample_crate",
                maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                jni_lib_name: "sample_crate_jni",
                jni_crate_path: "../../crates/sample_crate-jni",
                e2e_env: &env,
                capsule_types: &std::collections::HashMap::new(),
                test_documents_path: "../../test_documents",
            });
            assert!(
                output.contains(r#"environment("MY_SERVICE_ALLOW_PRIVATE_NETWORK", "true")"#),
                "build.gradle.kts ({dep_mode:?}) must forward e2e env vars to the test worker, got:\n{output}"
            );
        }
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
            let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
                kotlin_pkg_id: "dev.sample_crate",
                maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                jni_lib_name: "sample_crate_jni",
                jni_crate_path: "../../crates/sample_crate-jni",
                e2e_env: &std::collections::BTreeMap::new(),
                capsule_types: &std::collections::HashMap::new(),
                test_documents_path: "../../test_documents",
            });
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

    /// Regression: local-mode build.gradle.kts must declare a `testImplementation` for
    /// every `[crates.kotlin_android.capsule_types]` host package. Local mode compiles
    /// `packages/kotlin-android`'s wrapper sources directly into this project's `test`
    /// sourceSet (see `source_sets_block`), and those sources import the capsule's
    /// `host_type` (e.g. `io.github.treesitter.ktreesitter.Language`) whenever a function
    /// or method returns that capsule type. Without this dependency the Kotlin compiler
    /// fails with "Unresolved reference 'github'" (or whatever the host package's top
    /// segment is) on every local-mode e2e build, before a single test runs.
    #[test]
    fn build_gradle_kotlin_android_local_mode_declares_capsule_test_dependency() {
        let mut capsule_types = std::collections::HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            crate::core::config::HostCapsuleTypeConfig {
                host_type: "io.github.treesitter.ktreesitter.Language".to_string(),
                package: "io.github.tree-sitter:ktreesitter".to_string(),
                package_version: "0.25.0".to_string(),
                construct_expr: "io.github.treesitter.ktreesitter.Language({ptr})".to_string(),
                ..Default::default()
            },
        );
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Local,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &capsule_types,
            test_documents_path: "../../test_documents",
        });
        assert!(
            output.contains(r#"testImplementation("io.github.tree-sitter:ktreesitter:0.25.0")"#),
            "local-mode build.gradle.kts must declare a testImplementation for the capsule host package, got:\n{output}"
        );
    }

    /// Regression: registry-mode build.gradle.kts must NOT declare a `testImplementation`
    /// for capsule host packages -- the published AAR declares its capsule dependency as
    /// `api` (see `backends::kotlin_android::gen_build_gradle`), so it already resolves
    /// transitively through the `implementation(mavenCoordinate)` dependency. Duplicating
    /// it here would be redundant, not a compile blocker, but it would mask a real
    /// regression in the AAR's own `api` dependency if this test also passed.
    #[test]
    fn build_gradle_kotlin_android_registry_mode_omits_capsule_test_dependency() {
        let mut capsule_types = std::collections::HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            crate::core::config::HostCapsuleTypeConfig {
                host_type: "io.github.treesitter.ktreesitter.Language".to_string(),
                package: "io.github.tree-sitter:ktreesitter".to_string(),
                package_version: "0.25.0".to_string(),
                construct_expr: "io.github.treesitter.ktreesitter.Language({ptr})".to_string(),
                ..Default::default()
            },
        );
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Registry,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &capsule_types,
            test_documents_path: "../../test_documents",
        });
        assert!(
            !output.contains("io.github.tree-sitter:ktreesitter"),
            "registry-mode build.gradle.kts must not duplicate the capsule host package testImplementation, got:\n{output}"
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
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Registry,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });
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
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Registry,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });
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
            let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
                kotlin_pkg_id: "dev.sample_crate",
                maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                jni_lib_name: "sample_crate_jni",
                jni_crate_path: "../../crates/sample_crate-jni",
                e2e_env: &std::collections::BTreeMap::new(),
                capsule_types: &std::collections::HashMap::new(),
                test_documents_path: "../../test_documents",
            });
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
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate.samplellm.android",
            maven_coordinate: "dev.sample_crate:demo-client-android:1.0.0",
            dep_mode: crate::e2e::config::DependencyMode::Local,
            jni_lib_name: "demo_client_jni",
            jni_crate_path: "../../crates/demo-client-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });
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
            let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
                kotlin_pkg_id: "dev.sample_crate",
                maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
                dep_mode,
                jni_lib_name: "sample_crate_jni",
                jni_crate_path: "../../crates/sample_crate-jni",
                e2e_env: &std::collections::BTreeMap::new(),
                capsule_types: &std::collections::HashMap::new(),
                test_documents_path: "../../test_documents",
            });

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
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Local,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });

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
        let output = render_build_gradle_kotlin_android(&KotlinAndroidBuildGradleInputs {
            kotlin_pkg_id: "dev.sample_crate",
            maven_coordinate: "dev.sample_crate:sample_crate-android:5.0.0-rc.1",
            dep_mode: crate::e2e::config::DependencyMode::Registry,
            jni_lib_name: "sample_crate_jni",
            jni_crate_path: "../../crates/sample_crate-jni",
            e2e_env: &std::collections::BTreeMap::new(),
            capsule_types: &std::collections::HashMap::new(),
            test_documents_path: "../../test_documents",
        });

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
