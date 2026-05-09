use crate::scaffold_meta;
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;
use alef_core::template_versions::{maven, toolchain};

const JACKSON_VERSION: &str = "2.18.2";
use std::path::PathBuf;

pub(crate) fn scaffold_kotlin(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let kotlin_package = config.kotlin_package();
    let project_name = config.name.replace('-', "_");

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let kotlinx_coroutines = maven::KOTLINX_COROUTINES_CORE;
    let jna = maven::JNA;
    let junit_legacy = maven::JUNIT_LEGACY;
    let jvm_target = toolchain::JVM_TARGET;
    let kotlin_artifact_id = format!("{}-kotlin", config.name);

    // build.gradle.kts: Kotlin 2.x DSL — `compilerOptions` block replaces the
    // deprecated `kotlinOptions { jvmTarget }` form removed in Kotlin 2.1.
    let build_gradle = format!(
        r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {{
    `java-library`
    kotlin("jvm") version "{kotlin_plugin}"
    `maven-publish`
    id("org.jlleitschuh.gradle.ktlint") version "12.1.1"
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
    api("org.jspecify:jspecify:1.0.0")
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
            // Kotlin module compiles against the same on-disk sources.
            srcDir("../java/src/main/java")
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
    version.set("1.4.1")
    outputToConsole.set(true)
    ignoreFailures.set(false)
    filter {{
        exclude {{ entry -> entry.file.toString().contains("/packages/java/") }}
        exclude("**/build/**")
        exclude("**/generated/**")
    }}
}}

// JNA needs the native lib on java.library.path; default to the workspace
// `target/release` cargo output. Override with `-Pkb.lib.path=<dir>`.
tasks.withType<Test>().configureEach {{
    val libPath = (project.findProperty("kb.lib.path") as String?) ?: "${{rootDir}}/../../target/release"
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
        jackson = JACKSON_VERSION,
        kotlin_artifact_id = kotlin_artifact_id,
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
    implementation("{package}:your-lib-kt:{version}")
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

    let github_workflow = format!(
        r#"name: Kotlin

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: packages/kotlin
    steps:
      - uses: actions/checkout@v4
      - name: Set up JDK
        uses: actions/setup-java@v4
        with:
          java-version: "{jvm_target}"
          distribution: temurin
      - name: Set up Gradle
        uses: gradle/actions/setup-gradle@v4
      - name: Run Gradle build
        run: gradle build
      - name: Run Gradle tests
        run: gradle test
"#,
        jvm_target = jvm_target,
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
        GeneratedFile {
            path: PathBuf::from(".github/workflows/kotlin.yml"),
            content: github_workflow,
            generated_header: false,
        },
    ])
}
