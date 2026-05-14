use crate::scaffold_meta;
use alef_core::backend::GeneratedFile;
use alef_core::config::{KotlinTarget, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;
use alef_core::template_versions::{maven, toolchain};
use heck::ToPascalCase;

use std::path::PathBuf;

pub(crate) fn scaffold_kotlin(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if let Some(mode) = config.kotlin.as_ref().and_then(|k| k.mode.as_deref()) {
        return match mode {
            "android" => anyhow::bail!(
                "`[crates.kotlin] mode = \"android\"` was removed in alef 0.16. \
                 Use `Language::KotlinAndroid` (slug `\"kotlin_android\"`) and the \
                 `alef-backend-kotlin-android` crate instead."
            ),
            "kmp" => scaffold_kotlin_multiplatform(api, config),
            _ => scaffold_kotlin_jvm(api, config),
        };
    }
    if config.kotlin.as_ref().is_some_and(|k| k.target == KotlinTarget::Native) {
        return scaffold_kotlin_native(api, config);
    }
    if config
        .kotlin
        .as_ref()
        .is_some_and(|k| k.target == KotlinTarget::Multiplatform)
    {
        return scaffold_kotlin_multiplatform(api, config);
    }

    scaffold_kotlin_jvm(api, config)
}

fn scaffold_kotlin_jvm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let kotlin_package = config.kotlin_package();
    let project_name = config.name.replace('-', "_");

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let kotlinx_coroutines = maven::KOTLINX_COROUTINES_CORE;
    let jna = maven::JNA;
    let junit_legacy = maven::JUNIT_LEGACY;
    let jackson = maven::JACKSON;
    let jspecify = maven::JSPECIFY;
    let ktlint_gradle_plugin = maven::KTLINT_GRADLE_PLUGIN;
    let ktlint = maven::KTLINT;
    let jvm_target = toolchain::JVM_TARGET;
    let kotlin_artifact_id = format!("{}-kotlin", config.name);
    // Pascal-cased binding-class filename emitted by alef-backend-kotlin
    // (e.g. crate `kreuzberg` -> `Kreuzberg.kt`). The alef-emitted file is not
    // ktlint-clean (parameters on a single line, missing expression bodies),
    // so we exclude it here rather than reformatting in the backend.
    let binding_class = config.name.to_pascal_case();

    // build.gradle.kts: Kotlin 2.x DSL — `compilerOptions` block replaces the
    // deprecated `kotlinOptions { jvmTarget }` form removed in Kotlin 2.1.
    let build_gradle = format!(
        r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {{
  `java-library`
  kotlin("jvm") version "{kotlin_plugin}"
  `maven-publish`
  id("org.jlleitschuh.gradle.ktlint") version "{ktlint_gradle_plugin}"
}}

group = "{package}"
version = "{version}"

repositories {{
  mavenCentral()
}}

dependencies {{
  api("net.java.dev.jna:jna:{jna}")
  // Jackson is on the public surface because the alef-emitted Java records
  // include `@JsonProperty` annotations for serialization round-tripping.
  api("com.fasterxml.jackson.core:jackson-annotations:{jackson}")
  api("com.fasterxml.jackson.core:jackson-databind:{jackson}")
  api("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")
  // jspecify ships the `@Nullable` / `@NonNull` annotations referenced by the
  // alef-emitted Java facade; it must be on the api configuration so Kotlin
  // consumers see the annotations on cross-language types.
  api("org.jspecify:jspecify:{jspecify}")
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:{kotlinx_coroutines}")
  testImplementation("org.jetbrains.kotlin:kotlin-test:{kotlin_plugin}")
  testImplementation("junit:junit:{junit_legacy}")
}}

java {{
  sourceCompatibility = JavaVersion.VERSION_{jvm_target}
  targetCompatibility = JavaVersion.VERSION_{jvm_target}
}}

// Include the alef-emitted Java facade (sibling package) so the Kotlin object
// can call into the JNA-loaded native bridge. The Kotlin backend places its
// generated files in a sub-package (`<group>.kt`) to avoid colliding with the
// Java facade that uses the canonical `<group>` package.
sourceSets {{
  main {{
    java {{
      // Pull in the Java facade emitted by the alef Java backend so the
      // Kotlin module compiles against the same on-disk sources. The alef
      // Java backend writes to `packages/java/` (package-root layout), not
      // the Maven `src/main/java/` convention.
      srcDir("../java")
    }}
    kotlin {{
      // The alef Kotlin backend emits binding sources at the project root
      // (`packages/kotlin/`) rather than the Maven
      // `src/main/kotlin/` convention. Pull them in explicitly so they end up
      // in the compiled jar alongside any standard-layout sources.
      srcDir(".")
    }}
  }}
}}

kotlin {{
  compilerOptions {{
    jvmTarget.set(JvmTarget.JVM_{jvm_target})
  }}
}}

// ktlint configuration — see .editorconfig for details. We deliberately exclude
// the Java facade (which lives under `packages/java/`) and any build/generated
// directories: ktlint cannot lint pure-Java files, and the FFM/Panama bindings
// are kept in their own module.
ktlint {{
  version.set("{ktlint}")
  outputToConsole.set(true)
  ignoreFailures.set(false)
  filter {{
    exclude {{ entry -> entry.file.toString().contains("/packages/java/") }}
    exclude {{ entry -> entry.file.toString().endsWith("/{binding_class}.kt") }}
    exclude("**/build/**")
    exclude("**/generated/**")
  }}
}}

// Gradle 9.x flags an output-overlap validation error between
// :ktlintKotlinScriptCheck / :ktlintMainSourceSetCheck and :compileKotlin.
// Declare the explicit dependency so Gradle accepts the task graph.
tasks.matching {{ it.name == "compileKotlin" }}.configureEach {{
  mustRunAfter("ktlintKotlinScriptCheck")
  mustRunAfter("ktlintMainSourceSetCheck")
}}

// JNA needs the native lib on java.library.path; default to the workspace
// `target/release` cargo output. Override with `-Pnative.lib.path=<dir>`.
tasks.withType<Test>().configureEach {{
  val libPath = (project.findProperty("native.lib.path") as String?) ?: "$rootDir/../../target/release"
  systemProperty("jna.library.path", libPath)
  systemProperty("java.library.path", libPath)
  useJUnit()
}}

// Publish under a Kotlin-specific artifactId so consumers can disambiguate
// the Kotlin module from the sibling Java facade in the same Maven group.
publishing {{
  publications {{
    create<MavenPublication>("maven") {{
      artifactId = "{kotlin_artifact_id}"
      from(components["java"])
    }}
  }}
}}
"#,
        package = kotlin_package,
        version = version,
        jackson = jackson,
        jspecify = jspecify,
        ktlint_gradle_plugin = ktlint_gradle_plugin,
        ktlint = ktlint,
        kotlin_artifact_id = kotlin_artifact_id,
        binding_class = binding_class,
    );

    let settings_gradle = format!("rootProject.name = \"{project_name}\"\n");

    let gitignore = "build/\n.gradle/\n.idea/\n*.iml\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.kt]\nindent_style = space\nindent_size = 4\n\n[*.gradle.kts]\nindent_style = space\nindent_size = 2\n";

    let gradle_properties = "org.gradle.parallel=true\nkotlin.code.style=official\n";

    let readme = format!(
        r#"# {project_name}

{description}

## Installation

Add to your `build.gradle.kts`:

```kotlin
dependencies {{
    implementation("{package}:{kotlin_artifact_id}:{version}")
}}
```

## Building

```sh
gradle build
gradle test
```

## License

{license}
"#,
        project_name = project_name,
        description = meta.description,
        package = kotlin_package,
        kotlin_artifact_id = kotlin_artifact_id,
        version = version,
        license = meta.license,
    );

    // ktlint's `filename` rule requires a file with a single top-level
    // declaration to match the declaration name, so the object is named
    // `Sample` (matching `Sample.kt`) rather than including the project name.
    let sample_kotlin = format!(
        r#"package {package}.sample

// Sample usage of the generated Kotlin bindings.
// Replace with your actual API calls after code generation.

object Sample {{
    @JvmStatic
    fun main(args: Array<String>) {{
        println("Sample: {project_name} bindings loaded successfully")
    }}
}}
"#,
        package = kotlin_package,
        project_name = project_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/settings.gradle.kts"),
            content: settings_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/gradle.properties"),
            content: gradle_properties.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/README.md"),
            content: readme,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/src/main/kotlin/sample/Sample.kt"),
            content: sample_kotlin,
            generated_header: false,
        },
    ])
}

fn kotlin_native_def(config: &ResolvedCrateConfig) -> String {
    format!(
        "headers = {}\nheaderFilter = {}_*\nlinkerOpts = -L../../../target/release -l{}\n",
        config.ffi_header_name(),
        config.ffi_prefix(),
        config.ffi_lib_name()
    )
}

fn scaffold_kotlin_native(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let project_name = format!("{}-native", config.name);
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let crate_name = &config.name;
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-native
gradle build
```

## License

{license}
"#,
        description = meta.description,
        license = meta.license,
    );
    let build_gradle = format!(
        r#"plugins {{
    kotlin("multiplatform") version "{kotlin_plugin}"
}}

kotlin {{
    linuxX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}
}}
"#
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/settings.gradle.kts"),
            content: format!("rootProject.name = \"{project_name}\"\n"),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/kotlin-native/{crate_name}.def")),
            content: kotlin_native_def(config),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/.gitignore"),
            content: "build/\n.gradle/\n.idea/\n*.iml\n".to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/README.md"),
            content: readme,
            generated_header: false,
        },
    ])
}

fn scaffold_kotlin_multiplatform(
    _api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let project_name = format!("{}-kmp", config.name);
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let crate_name = &config.name;
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-mpp
gradle build
```

## License

{license}
"#,
        description = meta.description,
        license = meta.license,
    );
    let build_gradle = format!(
        r#"plugins {{
    kotlin("multiplatform") version "{kotlin_plugin}"
}}

kotlin {{
    jvm()

    linuxX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}

    macosArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}
}}
"#
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/settings.gradle.kts"),
            content: format!("rootProject.name = \"{project_name}\"\n"),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/kotlin-mpp/{crate_name}.def")),
            content: kotlin_native_def(config),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/.gitignore"),
            content: "build/\n.gradle/\n.idea/\n*.iml\n".to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/README.md"),
            content: readme,
            generated_header: false,
        },
    ])
}
