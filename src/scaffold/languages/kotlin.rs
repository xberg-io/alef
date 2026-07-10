use crate::core::backend::GeneratedFile;
use crate::core::config::{KotlinTarget, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions::{maven, toolchain};
use crate::scaffold::{parse_author, scaffold_meta};
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
    let kotlin_package_path = kotlin_package.replace('.', "/");
    let project_name = config.name.replace('-', "_");

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let kotlinx_coroutines = maven::KOTLINX_COROUTINES_CORE;
    let jna = maven::JNA;
    let junit_legacy = maven::JUNIT_LEGACY;
    let jackson = maven::JACKSON;
    let jackson_annotations = maven::JACKSON_ANNOTATIONS;
    let jspecify = maven::JSPECIFY;
    let ktlint_gradle_plugin = maven::KTLINT_GRADLE_PLUGIN;
    let ktlint = maven::KTLINT;
    let jvm_target = toolchain::KOTLIN_JVM_TARGET;
    let kotlin_artifact_id = format!("{}-kotlin", config.name);
    let binding_class = config.name.to_pascal_case();

    let vanniktech = maven::VANNIKTECH_MAVEN_PUBLISH;
    let repo_url = meta.configured_repository.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "Kotlin scaffold requires package metadata repository; set package_metadata.repository or scaffold.repository"
        )
    })?;
    if meta.authors.is_empty() {
        anyhow::bail!(
            "Kotlin scaffold requires package metadata authors; set package_metadata.authors or scaffold.authors"
        );
    }
    let license = meta.license.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Kotlin scaffold requires package metadata license; set package_metadata.license or scaffold.license"
        )
    })?;
    let scm = scm_urls(&repo_url);
    let license_url = match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    };
    let kt = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
    let licenses_block = if license_url.is_empty() {
        format!(
            "    licenses {{\n      license {{\n        name.set(\"{}\")\n      }}\n    }}\n",
            kt(license)
        )
    } else {
        format!(
            "    licenses {{\n      license {{\n        name.set(\"{}\")\n        url.set(\"{}\")\n      }}\n    }}\n",
            kt(license),
            kt(license_url)
        )
    };
    let developers_block = if meta.authors.is_empty() {
        String::new()
    } else {
        let devs: Vec<String> = meta
            .authors
            .iter()
            .map(|a| {
                let (name, email) = parse_author(a);
                format!(
                    "      developer {{\n        name.set(\"{}\")\n        email.set(\"{}\")\n      }}",
                    kt(name),
                    kt(email)
                )
            })
            .collect();
        format!("    developers {{\n{}\n    }}\n", devs.join("\n"))
    };
    let description = kt(&meta.description);
    let repo_url = kt(&repo_url);
    let scm_connection = kt(&scm.connection);
    let scm_developer_connection = kt(&scm.developer_connection);

    let build_gradle = format!(
        r#"import com.vanniktech.maven.publish.JavadocJar
import com.vanniktech.maven.publish.KotlinJvm
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

buildscript {{
  dependencies {{
    classpath("com.vanniktech:gradle-maven-publish-plugin:{vanniktech}")
  }}
}}

plugins {{
  `java-library`
  kotlin("jvm") version "{kotlin_plugin}"
  id("com.vanniktech.maven.publish") version "{vanniktech}"
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
  api("com.fasterxml.jackson.core:jackson-annotations:{jackson_annotations}")
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

// Publish to Maven Central via the vanniktech plugin: signs all publications
// and uploads with publishingType=AUTOMATIC, so `publishAndReleaseToMavenCentral`
// auto-releases the Central Portal deployment (the bare `maven-publish` plugin
// can only stage, leaving the artifact unreleased). The Kotlin-specific
// artifactId disambiguates this module from the sibling Java facade in the same
// Maven group; the version is inherited from the top-level `version` above
// (kept current by `alef sync-versions`), so it is omitted from `coordinates`.
mavenPublishing {{
  configure(
    KotlinJvm(
      javadocJar = JavadocJar.Empty(),
      sourcesJar = true,
    ),
  )

  publishToMavenCentral()
  signAllPublications()

  coordinates(
    groupId = "{package}",
    artifactId = "{kotlin_artifact_id}",
  )

  pom {{
    name.set("{kotlin_artifact_id}")
    description.set("{description}")
    url.set("{repo_url}")
{licenses_block}{developers_block}    scm {{
      url.set("{repo_url}")
      connection.set("{scm_connection}")
      developerConnection.set("{scm_developer_connection}")
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
        scm_connection = scm_connection,
        scm_developer_connection = scm_developer_connection,
    );

    let settings_gradle = format!("rootProject.name = \"{project_name}\"\n");

    let gitignore = "build/\n.gradle/\n.idea/\n*.iml\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\n\n\
[*.kt]\nindent_style = space\nindent_size = 4\n\
ktlint_standard_class-signature = disabled\n\
ktlint_standard_function-signature = disabled\n\
ktlint_standard_function-expression-body = disabled\n\
ktlint_standard_no-empty-class-body = disabled\n\
ktlint_standard_no-empty-first-line-in-method-block = disabled\n\
ktlint_standard_indent = disabled\n\
ktlint_standard_string-template-indent = disabled\n\
ktlint_standard_filename = disabled\n\
ktlint_standard_multiline-expression-wrapping = disabled\n\
ktlint_standard_chain-method-continuation = disabled\n\
ktlint_standard_multiline-if-else = disabled\n\
ktlint_standard_parameter-list-wrapping = disabled\n\
ktlint_standard_argument-list-wrapping = disabled\n\
ktlint_standard_max-line-length = disabled\n\
ktlint_standard_function-literal = disabled\n\
ktlint_standard_trailing-comma-on-call-site = disabled\n\
ktlint_standard_trailing-comma-on-declaration-site = disabled\n\
ktlint_standard_statement-wrapping = disabled\n\n\
[*.gradle.kts]\nindent_style = space\nindent_size = 2\n\
ktlint_standard_class-signature = disabled\n\
ktlint_standard_function-signature = disabled\n\
ktlint_standard_function-expression-body = disabled\n\
ktlint_standard_no-empty-class-body = disabled\n\
ktlint_standard_no-empty-first-line-in-method-block = disabled\n\
ktlint_standard_indent = disabled\n\
ktlint_standard_string-template-indent = disabled\n\
ktlint_standard_filename = disabled\n\
ktlint_standard_multiline-expression-wrapping = disabled\n\
ktlint_standard_chain-method-continuation = disabled\n\
ktlint_standard_multiline-if-else = disabled\n\
ktlint_standard_parameter-list-wrapping = disabled\n\
ktlint_standard_argument-list-wrapping = disabled\n\
ktlint_standard_max-line-length = disabled\n\
ktlint_standard_function-literal = disabled\n\
ktlint_standard_trailing-comma-on-call-site = disabled\n\
ktlint_standard_trailing-comma-on-declaration-site = disabled\n\
ktlint_standard_statement-wrapping = disabled\n";

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
        license = license,
    );

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
            path: PathBuf::from(format!(
                "packages/kotlin/src/main/kotlin/{kotlin_package_path}/sample/Sample.kt"
            )),
            content: sample_kotlin,
            generated_header: false,
        },
    ])
}

struct ScmUrls {
    connection: String,
    developer_connection: String,
}

fn scm_urls(repository: &str) -> ScmUrls {
    let normalized = repository.trim_end_matches(".git");
    let without_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or(normalized);
    let (host, path) = without_scheme.split_once('/').unwrap_or((without_scheme, ""));
    let suffix = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}.git")
    };

    ScmUrls {
        connection: format!("scm:git:git://{host}{suffix}"),
        developer_connection: format!("scm:git:ssh://git@{host}{suffix}"),
    }
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
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-native
gradle build
```
"#,
        description = meta.description,
    ) + &license_section;
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
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-mpp
gradle build
```
"#,
        description = meta.description,
    ) + &license_section;
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
