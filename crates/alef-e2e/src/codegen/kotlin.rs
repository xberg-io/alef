//! Kotlin e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates `packages/kotlin/src/test/kotlin/<package>/<Name>Test.kt` files
//! from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_kotlin, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup, HttpFixture, ValidationErrorExpectation};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::{maven, toolchain};
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Kotlin e2e code generator.
pub struct KotlinE2eCodegen;

impl E2eCodegen for KotlinE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[alef_core::ir::TypeDef],
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
        let kotlin_pkg = e2e_config.resolve_package("kotlin");
        let pkg_name = kotlin_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.clone());

        // Resolve Kotlin package for generated tests.
        let _kotlin_pkg_path = kotlin_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/kotlin".to_string());
        let kotlin_version = kotlin_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());
        let kotlin_pkg_id = config.kotlin_package();

        // Detect whether any fixture needs the mock-server (HTTP fixtures or
        // fixtures with a mock_response/mock_responses). When present, emit a
        // JUnit Platform LauncherSessionListener that spawns the mock-server
        // before any test runs and a META-INF/services SPI manifest registering
        // it. Mirrors the Java e2e pattern exactly.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Generate build.gradle.kts.
        files.push(GeneratedFile {
            path: output_base.join("build.gradle.kts"),
            content: render_build_gradle(
                &pkg_name,
                &kotlin_pkg_id,
                &kotlin_version,
                e2e_config.dep_mode,
                needs_mock_server,
            ),
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
                content: render_mock_server_listener_kt(&kotlin_pkg_id),
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
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &HashSet::new(),
        );

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

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_file_name = format!("{}Test.kt", sanitize_filename(&group.category).to_upper_camel_case());
            let content = render_test_file(
                &group.category,
                &active,
                &class_name,
                &function_name,
                &kotlin_pkg_id,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
                e2e_config,
                &type_enum_fields,
            );
            files.push(GeneratedFile {
                path: test_base.join(class_file_name),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "kotlin"
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `ty` is a `Named(T)` reference (or `Optional<Named(T)>`)
/// where `T` is **not** a known struct name. Such fields are enum-typed and
/// must route through `.getValue()` in generated assertions.
fn is_enum_typed(ty: &alef_core::ir::TypeRef, struct_names: &HashSet<&str>) -> bool {
    use alef_core::ir::TypeRef;
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

pub(crate) fn render_build_gradle(
    pkg_name: &str,
    kotlin_pkg_id: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
    needs_mock_server: bool,
) -> String {
    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            // Registry mode: maven central with group:artifact:version
            format!(r#"    testImplementation("{kotlin_pkg_id}:{pkg_name}:{pkg_version}")"#)
        }
        crate::config::DependencyMode::Local => {
            // Local mode: reference the kotlin binding's built jar. The Kotlin
            // module is produced by `gradle build` under
            // `packages/kotlin/build/libs/<jar_name>-<version>.jar`, not by cargo.
            // We must also pull in the binding's runtime dependencies (JNA,
            // Jackson, jspecify, kotlinx-coroutines) since `files()` does not
            // resolve transitive metadata.
            let jar_name = pkg_name.rsplit(':').next().unwrap_or(pkg_name).replace('-', "_");
            let jna = maven::JNA;
            let jackson = maven::JACKSON_E2E;
            let jspecify = maven::JSPECIFY;
            let coroutines = maven::KOTLINX_COROUTINES_CORE;
            format!(
                r#"    testImplementation(files("../../packages/kotlin/build/libs/{jar_name}-{pkg_version}.jar"))
    testImplementation("net.java.dev.jna:jna:{jna}")
    testImplementation("com.fasterxml.jackson.core:jackson-annotations:{jackson}")
    testImplementation("com.fasterxml.jackson.core:jackson-databind:{jackson}")
    testImplementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")
    testImplementation("org.jspecify:jspecify:{jspecify}")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:{coroutines}")"#
            )
        }
    };

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let junit = maven::JUNIT;
    let jackson = maven::JACKSON_E2E;
    let jvm_target = toolchain::JVM_TARGET;
    let launcher_dep = if needs_mock_server {
        format!(r#"    testImplementation("org.junit.platform:junit-platform-launcher:{junit}")"#)
    } else {
        String::new()
    };
    format!(
        r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {{
    kotlin("jvm") version "{kotlin_plugin}"
    java
}}

group = "{kotlin_pkg_id}"
version = "0.1.0"

java {{
    sourceCompatibility = JavaVersion.VERSION_{jvm_target}
    targetCompatibility = JavaVersion.VERSION_{jvm_target}
}}

kotlin {{
    compilerOptions {{
        jvmTarget.set(JvmTarget.JVM_{jvm_target})
    }}
}}

repositories {{
    mavenCentral()
}}

dependencies {{
{dep_block}
    testImplementation("org.junit.jupiter:junit-jupiter-api:{junit}")
    testImplementation("org.junit.jupiter:junit-jupiter-engine:{junit}")
{launcher_dep}
    testImplementation("com.fasterxml.jackson.core:jackson-databind:{jackson}")
    testImplementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")
    testImplementation(kotlin("test"))
}}

tasks.test {{
    useJUnitPlatform()
    val libPath = System.getProperty("native.lib.path") ?: "${{rootDir}}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)
    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/.
    workingDir = file("${{rootDir}}/../../test_documents")
}}
"#
    )
}

/// Render the JUnit Platform `LauncherSessionListener` that spawns the
/// mock-server binary once per launcher session and tears it down on close.
///
/// Mirrors the Java `MockServerListener.java` — same logic, idiomatic Kotlin.
/// The URL is exposed via `System.setProperty("mockServerUrl", url)`;
/// generated test bodies read `System.getenv("MOCK_SERVER_URL")` (which the
/// listener also honours to skip spawning when the caller already has the
/// server running).
pub(crate) fn render_mock_server_listener_kt(kotlin_pkg_id: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    format!(
        r#"{header}package {kotlin_pkg_id}.e2e

import java.io.BufferedReader
import java.io.IOException
import java.io.InputStreamReader
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import java.nio.file.Paths
import java.util.regex.Pattern
import org.junit.platform.launcher.LauncherSession
import org.junit.platform.launcher.LauncherSessionListener

/**
 * Spawns the mock-server binary once per JUnit launcher session and
 * exposes its URL as the `mockServerUrl` system property. Generated
 * test bodies read the property (with `MOCK_SERVER_URL` env-var
 * fallback) so tests can run via plain `./gradlew test` without any
 * external mock-server orchestration. Mirrors the Ruby spec_helper /
 * Python conftest spawn pattern. Honors a pre-set MOCK_SERVER_URL by
 * skipping the spawn entirely.
 */
class MockServerListener : LauncherSessionListener {{
    private var mockServer: Process? = null

    override fun launcherSessionOpened(session: LauncherSession) {{
        val preset = System.getenv("MOCK_SERVER_URL")
        if (!preset.isNullOrEmpty()) {{
            System.setProperty("mockServerUrl", preset)
            return
        }}
        val repoRoot = locateRepoRoot()
            ?: error("MockServerListener: could not locate repo root (looked for fixtures/ in ancestors of ${{System.getProperty("user.dir")}})")
        val binName = if (System.getProperty("os.name", "").lowercase().contains("win")) "mock-server.exe" else "mock-server"
        val bin = repoRoot.resolve("e2e").resolve("rust").resolve("target").resolve("release").resolve(binName).toFile()
        val fixturesDir = repoRoot.resolve("fixtures").toFile()
        check(bin.exists()) {{
            "MockServerListener: mock-server binary not found at $bin — run: cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release"
        }}
        val pb = ProcessBuilder(bin.absolutePath, fixturesDir.absolutePath)
            .redirectErrorStream(false)
        val server = try {{
            pb.start()
        }} catch (e: IOException) {{
            throw IllegalStateException("MockServerListener: failed to start mock-server", e)
        }}
        mockServer = server
        // Read until we see MOCK_SERVER_URL= and optionally MOCK_SERVERS=.
        // Cap the loop so a misbehaving mock-server cannot block indefinitely.
        val stdout = BufferedReader(InputStreamReader(server.inputStream, StandardCharsets.UTF_8))
        var url: String? = null
        try {{
            for (i in 0 until 16) {{
                val line = stdout.readLine() ?: break
                when {{
                    line.startsWith("MOCK_SERVER_URL=") -> {{
                        url = line.removePrefix("MOCK_SERVER_URL=").trim()
                    }}
                    line.startsWith("MOCK_SERVERS=") -> {{
                        val jsonVal = line.removePrefix("MOCK_SERVERS=").trim()
                        System.setProperty("mockServers", jsonVal)
                        // Parse JSON map of fixture_id -> url and expose as system properties.
                        val p = Pattern.compile(""""([^"]+)":"([^"]+)"""")
                        val matcher = p.matcher(jsonVal)
                        while (matcher.find()) {{
                            System.setProperty("mockServer.${{matcher.group(1)}}", matcher.group(2))
                        }}
                        break
                    }}
                    url != null -> break
                }}
            }}
        }} catch (e: IOException) {{
            server.destroyForcibly()
            throw IllegalStateException("MockServerListener: failed to read mock-server stdout", e)
        }}
        if (url.isNullOrEmpty()) {{
            server.destroyForcibly()
            error("MockServerListener: mock-server did not emit MOCK_SERVER_URL")
        }}
        // TCP-readiness probe: ensure axum::serve is accepting before tests start.
        // The mock-server binds the TcpListener synchronously then prints the URL
        // before tokio::spawn(axum::serve(...)) is polled, so under Gradle parallel
        // mode tests can race startup. Poll-connect (max 5s, 50ms backoff) until success.
        val healthUri = java.net.URI.create(url)
        val host = healthUri.host
        val port = healthUri.port
        val deadline = System.nanoTime() + 5_000_000_000L
        while (System.nanoTime() < deadline) {{
            try {{
                java.net.Socket().use {{ s ->
                    s.connect(java.net.InetSocketAddress(host, port), 100)
                    break
                }}
            }} catch (_: java.io.IOException) {{
                try {{ Thread.sleep(50) }} catch (ie: InterruptedException) {{ Thread.currentThread().interrupt(); break }}
            }}
        }}
        System.setProperty("mockServerUrl", url)
        // Drain remaining stdout/stderr in daemon threads so a full pipe
        // does not block the child.
        Thread {{ drain(stdout) }}.also {{ it.isDaemon = true }}.start()
        Thread {{ drain(BufferedReader(InputStreamReader(server.errorStream, StandardCharsets.UTF_8))) }}.also {{ it.isDaemon = true }}.start()
    }}

    override fun launcherSessionClosed(session: LauncherSession) {{
        val server = mockServer ?: return
        try {{ server.outputStream.close() }} catch (_: IOException) {{}}
        try {{
            if (!server.waitFor(2, java.util.concurrent.TimeUnit.SECONDS)) {{
                server.destroyForcibly()
            }}
        }} catch (ie: InterruptedException) {{
            Thread.currentThread().interrupt()
            server.destroyForcibly()
        }}
    }}

    companion object {{
        private fun locateRepoRoot(): Path? {{
            var dir: Path? = Paths.get("").toAbsolutePath()
            while (dir != null) {{
                if (dir.resolve("fixtures").toFile().isDirectory
                    && dir.resolve("e2e").toFile().isDirectory) {{
                    return dir
                }}
                dir = dir.parent
            }}
            return null
        }}

        private fun drain(reader: BufferedReader) {{
            try {{
                val buf = CharArray(1024)
                while (reader.read(buf) >= 0) {{ /* drain */ }}
            }} catch (_: IOException) {{}}
        }}
    }}
}}
"#
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
) -> String {
    render_test_file_inner(
        category,
        fixtures,
        class_name,
        function_name,
        kotlin_pkg_id,
        result_var,
        args,
        options_type,
        field_resolver,
        result_is_simple,
        enum_fields,
        e2e_config,
        type_enum_fields,
        false,
    )
}

/// Variant of [`render_test_file`] used by the kotlin_android backend.
///
/// `kotlin_android_style = true` shifts two emission decisions:
///
/// 1. Every emitted `@Test` body is wrapped in `runBlocking { ... }` so the
///    suspend-only public API (the kotlin_android AAR exposes most
///    extraction entry points as `suspend fun`) can be invoked from
///    JUnit's non-suspend `@Test` methods. JVM Kotlin tests keep the
///    previous behaviour and only wrap when a `client_factory` is in play.
/// 2. Option-returning APIs are treated as Kotlin nullable `T?` (the
///    kotlin-android wrapper unwraps Java `Optional<T>` to `T?` at the
///    boundary), so `is_empty` / `not_empty` assertions on a bare option
///    result emit `== null` / `!= null` instead of `.isEmpty` /
///    `.isPresent`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_test_file_android(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
) -> String {
    render_test_file_inner(
        category,
        fixtures,
        class_name,
        function_name,
        kotlin_pkg_id,
        result_var,
        args,
        options_type,
        field_resolver,
        result_is_simple,
        enum_fields,
        e2e_config,
        type_enum_fields,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file_inner(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    kotlin_pkg_id: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    kotlin_android_style: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls. Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    let _ = writeln!(out, "package {kotlin_pkg_id}.e2e");
    let _ = writeln!(out);

    // Detect if any fixture in this group is an HTTP server test.
    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());

    // Detect if any non-HTTP fixture uses a client_factory (coroutine-based client).
    // When true, test functions must use `= runBlocking { ... }` to call suspend fns.
    let has_client_factory_fixtures = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
        let per_call_factory = cc.overrides.get("kotlin").and_then(|o| o.client_factory.as_deref());
        let global_factory = e2e_config
            .call
            .overrides
            .get("kotlin")
            .and_then(|o| o.client_factory.as_deref());
        per_call_factory.or(global_factory).is_some()
    });

    // Collect every (per-call) options_type referenced by fixtures in this file.
    // Per-call kotlin overrides win over the file-level options_type passed in.
    // Each entry is a json_object arg's options_type — we need to import each one.
    let mut per_fixture_options_types: HashSet<String> = HashSet::new();
    for f in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
        let call_overrides = cc.overrides.get("kotlin");
        let effective_opts: Option<String> = call_overrides
            .and_then(|o| o.options_type.clone())
            .or_else(|| options_type.map(|s| s.to_string()))
            .or_else(|| {
                for cand in ["csharp", "c", "go", "php", "python"] {
                    if let Some(o) = cc.overrides.get(cand) {
                        if let Some(t) = &o.options_type {
                            return Some(t.clone());
                        }
                    }
                }
                None
            });
        if let Some(opts) = effective_opts {
            // Prefer the per-call args (which carry the correct arg_type + field for the
            // resolved call); fall back to the file-level args only when the call has none.
            let fixture_args = if cc.args.is_empty() { args } else { cc.args.as_slice() };
            // Import the options type if the fixture either supplies a json_object value
            // (deserialised via ObjectMapper) OR has an *optional* json_object arg with
            // no value — the generator emits `OptionsType.builder().build()` in that
            // case to keep the call arity correct.
            let needs_opts_type = fixture_args.iter().any(|arg| {
                if arg.arg_type != "json_object" {
                    return false;
                }
                let v = super::resolve_field(&f.input, &arg.field);
                !v.is_null() || arg.optional
            });
            if needs_opts_type {
                per_fixture_options_types.insert(opts.to_string());
            }
        }
    }
    let needs_object_mapper_for_options = !per_fixture_options_types.is_empty();
    // Also need ObjectMapper when a handle arg has a non-null config.
    let needs_object_mapper_for_handle = fixtures.iter().any(|f| {
        args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });
    // HTTP fixtures always need ObjectMapper for JSON body comparison.
    let needs_object_mapper = needs_object_mapper_for_options || needs_object_mapper_for_handle || has_http_fixtures;

    let _ = writeln!(out, "import org.junit.jupiter.api.Test");
    let _ = writeln!(out, "import kotlin.test.assertEquals");
    let _ = writeln!(out, "import kotlin.test.assertTrue");
    let _ = writeln!(out, "import kotlin.test.assertFalse");
    let _ = writeln!(out, "import kotlin.test.assertFailsWith");
    if has_client_factory_fixtures || kotlin_android_style {
        let _ = writeln!(out, "import kotlinx.coroutines.runBlocking");
    }
    // Effective binding package for FQN imports. When the binding `class_name` is
    // not fully-qualified, fall back to `kotlin_pkg_id` — the kotlin binding emits
    // top-level typealiases at that package (e.g. `package com.github.kreuzberg_dev`)
    // while the test files live at `<kotlin_pkg_id>.e2e`. Child packages do NOT
    // import their parent's symbols implicitly, so explicit imports are required.
    let binding_pkg_for_imports: String = if !import_path.is_empty() {
        import_path
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_else(|| kotlin_pkg_id.to_string())
    } else {
        kotlin_pkg_id.to_string()
    };
    // Only import the binding class when there are non-HTTP fixtures that call it.
    let has_call_fixtures = fixtures.iter().any(|f| !f.is_http_test());
    if has_call_fixtures {
        if !import_path.is_empty() {
            let _ = writeln!(out, "import {import_path}");
        } else if !class_name.is_empty() {
            let _ = writeln!(out, "import {binding_pkg_for_imports}.{class_name}");
        }
    }
    if needs_object_mapper {
        let _ = writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper");
        let _ = writeln!(out, "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module");
    }
    // Import every options type referenced by per-call kotlin overrides in this file.
    // Options-type imports are needed for both ObjectMapper deserialisation and for
    // optional-arg defaults emitted as `OptionsType.builder().build()`.
    if has_call_fixtures {
        let mut sorted_opts: Vec<&String> = per_fixture_options_types.iter().collect();
        sorted_opts.sort();
        for opts_type in sorted_opts {
            let _ = writeln!(out, "import {binding_pkg_for_imports}.{opts_type}");
        }
    }
    // Import CrawlConfig when handle args need JSON deserialization.
    if needs_object_mapper_for_handle {
        let _ = writeln!(out, "import {binding_pkg_for_imports}.CrawlConfig");
    }
    // Import BatchBytesItem / BatchFileItem when any fixture has a batch-item
    // array arg (element_type) — the test code constructs these directly.
    let mut batch_elem_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
        let fixture_args = if cc.args.is_empty() { args } else { cc.args.as_slice() };
        for arg in fixture_args.iter() {
            if arg.arg_type != "json_object" {
                continue;
            }
            let v = super::resolve_field(&f.input, &arg.field);
            if !v.is_array() {
                continue;
            }
            if let Some(elem) = &arg.element_type {
                if elem == "BatchBytesItem" || elem == "BatchFileItem" {
                    batch_elem_imports.insert(elem.clone());
                }
            }
        }
    }
    for elem in &batch_elem_imports {
        let _ = writeln!(out, "import {binding_pkg_for_imports}.{elem}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "class {test_class_name} {{");

    if needs_object_mapper {
        let _ = writeln!(out);
        let _ = writeln!(out, "    companion object {{");
        let _ = writeln!(
            out,
            "        private val MAPPER = ObjectMapper().registerModule(Jdk8Module()).setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)"
        );
        let _ = writeln!(out, "    }}");
    }

    for fixture in fixtures {
        render_test_method(
            &mut out,
            fixture,
            simple_class,
            function_name,
            result_var,
            args,
            options_type,
            field_resolver,
            result_is_simple,
            enum_fields,
            e2e_config,
            type_enum_fields,
            kotlin_android_style,
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}

// ---------------------------------------------------------------------------
// HTTP server test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits JUnit 5 `@Test fun testFoo()` blocks using
/// `java.net.http.HttpClient` against `System.getenv("MOCK_SERVER_URL")`.
pub(crate) struct KotlinTestClientRenderer;

impl client::TestClientRenderer for KotlinTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "kotlin"
    }

    fn sanitize_test_name(&self, id: &str) -> String {
        sanitize_ident(id).to_upper_camel_case()
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    fun test{fn_name}() {{");
        let _ = writeln!(out, "        // {description}");
        if let Some(reason) = skip_reason {
            let escaped = escape_kotlin(reason);
            let _ = writeln!(
                out,
                "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"{escaped}\")"
            );
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_path = ctx.path;

        // Java's HttpClient restricts certain headers that cannot be set programmatically.
        const JAVA_RESTRICTED_HEADERS: &[&str] = &["connection", "content-length", "expect", "host", "upgrade"];

        let _ = writeln!(
            out,
            "        val baseUrl = System.getenv(\"MOCK_SERVER_URL\") ?: \"http://localhost:8080\""
        );
        let _ = writeln!(out, "        val uri = java.net.URI.create(\"$baseUrl{fixture_path}\")");

        let body_publisher = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_kotlin(&json);
            format!("java.net.http.HttpRequest.BodyPublishers.ofString(\"{escaped}\")")
        } else {
            "java.net.http.HttpRequest.BodyPublishers.noBody()".to_string()
        };

        let _ = writeln!(out, "        val builder = java.net.http.HttpRequest.newBuilder(uri)");
        let _ = writeln!(out, "            .method(\"{method}\", {body_publisher})");

        // Content-Type header when there is a body.
        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let _ = writeln!(out, "            .header(\"Content-Type\", \"{content_type}\")");
        }

        // Explicit request headers (sorted for deterministic output).
        let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
        header_pairs.sort_by_key(|(k, _)| k.as_str());
        for (name, value) in &header_pairs {
            if JAVA_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_kotlin(name);
            let escaped_value = escape_kotlin(value);
            let _ = writeln!(out, "            .header(\"{escaped_name}\", \"{escaped_value}\")");
        }

        // Cookies as a single Cookie header.
        if !ctx.cookies.is_empty() {
            let mut cookie_pairs: Vec<(&String, &String)> = ctx.cookies.iter().collect();
            cookie_pairs.sort_by_key(|(k, _)| k.as_str());
            let cookie_str: Vec<String> = cookie_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_kotlin(&cookie_str.join("; "));
            let _ = writeln!(out, "            .header(\"Cookie\", \"{cookie_header}\")");
        }

        let _ = writeln!(
            out,
            "        val {} = java.net.http.HttpClient.newHttpClient()",
            ctx.response_var
        );
        let _ = writeln!(
            out,
            "            .send(builder.build(), java.net.http.HttpResponse.BodyHandlers.ofString())"
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "        assertEquals({status}, {response_var}.statusCode(), \"status code mismatch\")"
        );
    }

    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_kotlin(name);
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isPresent, \"header {escaped_name} should be present\")"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "        assertFalse({response_var}.headers().firstValue(\"{escaped_name}\").isPresent, \"header {escaped_name} should be absent\")"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").matches(\"[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\"), \"header {escaped_name} should be a UUID\")"
                );
            }
            exact => {
                let escaped_value = escape_kotlin(exact);
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").contains(\"{escaped_value}\"), \"header {escaped_name} mismatch\")"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_kotlin(&json_str);
                let _ = writeln!(out, "        val bodyJson = MAPPER.readTree({response_var}.body())");
                let _ = writeln!(out, "        val expectedJson = MAPPER.readTree(\"{escaped}\")");
                let _ = writeln!(out, "        assertEquals(expectedJson, bodyJson, \"body mismatch\")");
            }
            serde_json::Value::String(s) => {
                let escaped = escape_kotlin(s);
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\")"
                );
            }
            other => {
                let escaped = escape_kotlin(&other.to_string());
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\")"
                );
            }
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "        val _partialTree = MAPPER.readTree({response_var}.body())");
            for (key, val) in obj {
                let escaped_key = escape_kotlin(key);
                match val {
                    serde_json::Value::String(s) => {
                        let escaped_val = escape_kotlin(s);
                        let _ = writeln!(
                            out,
                            "        assertEquals(\"{escaped_val}\", _partialTree.path(\"{escaped_key}\").asText(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    serde_json::Value::Bool(b) => {
                        let _ = writeln!(
                            out,
                            "        assertEquals({b}, _partialTree.path(\"{escaped_key}\").asBoolean(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    serde_json::Value::Number(n) => {
                        let _ = writeln!(
                            out,
                            "        assertEquals({n}, _partialTree.path(\"{escaped_key}\").numberValue(), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                    other => {
                        let json_str = serde_json::to_string(other).unwrap_or_default();
                        let escaped_val = escape_kotlin(&json_str);
                        let _ = writeln!(
                            out,
                            "        assertEquals(MAPPER.readTree(\"{escaped_val}\"), _partialTree.path(\"{escaped_key}\"), \"partial body field '{escaped_key}' mismatch\")"
                        );
                    }
                }
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "        val _veTree = MAPPER.readTree({response_var}.body())");
        let _ = writeln!(out, "        val _veErrors = _veTree.path(\"errors\")");
        for ve in errors {
            let escaped_msg = escape_kotlin(&ve.msg);
            let _ = writeln!(
                out,
                "        assertTrue((0 until _veErrors.size()).any {{ _veErrors.get(it).path(\"msg\").asText().contains(\"{escaped_msg}\") }}, \"expected validation error containing: {escaped_msg}\")"
            );
        }
    }
}

/// Render a JUnit 5 `@Test` method for an HTTP server fixture via the shared driver.
///
/// HTTP 101 (WebSocket upgrade) is emitted as a skip stub because Java's
/// `HttpClient` cannot handle protocol-switch responses (throws `EOFException`).
fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) — java.net.http.HttpClient cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_ident(&fixture.id).to_upper_camel_case();
        let description = &fixture.description;
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    fun test{method_name}() {{");
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"Skipped: Java HttpClient cannot handle 101 Switching Protocols responses\")"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    client::http_call::render_http_test(out, &KotlinTestClientRenderer, fixture);
}

#[allow(clippy::too_many_arguments)]
fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    class_name: &str,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    kotlin_android_style: bool,
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let lang = "kotlin";
    let call_overrides = call_config.overrides.get(lang);

    // Check for client_factory — when set, use instance-method call style.
    // Falls back to the global `[e2e.call.overrides.kotlin]` `client_factory` when
    // a per-call override is absent, matching the dart/swift renderers.
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let effective_result_var = &call_config.result_var;
    let effective_args = &call_config.args;
    let function_name = effective_function_name.as_str();
    let result_var = effective_result_var.as_str();
    let args: &[crate::config::ArgMapping] = effective_args.as_slice();
    // Resolve per-fixture options_type: prefer the kotlin call override, fall back
    // to class-level, then to any other language's options_type for the same call.
    // The Kotlin module re-exports Java facade types unchanged, so a type name declared
    // by csharp/c/go/php/python applies equally to Kotlin without an explicit override.
    let effective_options_type: Option<String> = call_overrides
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()))
        .or_else(|| {
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = call_config.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        return Some(t.clone());
                    }
                }
            }
            None
        });
    let options_type = effective_options_type.as_deref();

    // Resolve per-fixture result_is_simple: prefer the kotlin override, then the
    // class-level default, then any sibling language override (java/csharp/go).
    // The Kotlin facade shares its return-type shape with the Java facade, so a
    // declaration in any of those bindings applies to Kotlin too.
    let effective_result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple)
        || call_config.result_is_simple
        || result_is_simple
        || ["java", "csharp", "go"]
            .iter()
            .any(|cand| call_config.overrides.get(*cand).is_some_and(|o| o.result_is_simple));
    let result_is_simple = effective_result_is_simple;

    // Resolve per-fixture result_is_option: prefer the kotlin override, then the
    // call-level default. When set the function returns `T?` and bare-result
    // emptiness assertions must use a null-check instead of `.isEmpty()`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming = crate::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming);
    let collect_snippet = if is_streaming && !expects_error {
        crate::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet("kotlin", result_var, "chunks")
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Check if this test needs ObjectMapper deserialization for json_object args.
    // Uses `resolve_field` so that `field = "input"` resolves to the whole fixture
    // input (and not a nested key called "input"), matching dart/swift behavior.
    let needs_deser = options_type.is_some()
        && args
            .iter()
            .any(|arg| arg.arg_type == "json_object" && !super::resolve_field(&fixture.input, &arg.field).is_null());

    // Merge per-call kotlin enum_fields (HashMap key = field path, value = enum type name)
    // into the global fields_enum set so that call-specific enum-typed result fields
    // (e.g. `status` on BatchObject) route through `.getValue()` in assertions even
    // when absent from the global `fields_enum` list.  Mirrors the Java codegen at
    // codegen/java.rs where per-call overrides are merged before assertion rendering.
    //
    // Additionally, auto-detect enum-typed fields by looking up the call's result type
    // in `type_enum_fields` (built from the IR TypeDef list). This handles the common
    // case where a field's Rust type is a `Named(EnumName)` that was never explicitly
    // listed in the alef.toml `enum_fields` table.
    let effective_enum_fields: std::borrow::Cow<HashSet<String>> = {
        // Resolve the result type name for this call. Prefer the kotlin override, then
        // java, then c — the Kotlin facade re-exports Java facade types unchanged.
        let result_type_name: Option<&str> = call_overrides
            .and_then(|co| co.result_type.as_deref())
            .or_else(|| call_config.overrides.get("java").and_then(|o| o.result_type.as_deref()))
            .or_else(|| call_config.overrides.get("c").and_then(|o| o.result_type.as_deref()));
        let auto_enum_fields: Option<&HashSet<String>> = result_type_name.and_then(|name| type_enum_fields.get(name));
        let has_per_call = call_overrides.is_some_and(|co| !co.enum_fields.is_empty());
        let has_auto = auto_enum_fields.is_some_and(|f| !f.is_empty());
        if has_per_call || has_auto {
            let mut merged = enum_fields.clone();
            if let Some(co) = call_overrides {
                merged.extend(co.enum_fields.keys().cloned());
            }
            if let Some(auto_fields) = auto_enum_fields {
                merged.extend(auto_fields.iter().cloned());
            }
            std::borrow::Cow::Owned(merged)
        } else {
            std::borrow::Cow::Borrowed(enum_fields)
        }
    };
    let enum_fields: &HashSet<String> = &effective_enum_fields;

    let _ = writeln!(out, "    @Test");
    if client_factory.is_some() || kotlin_android_style {
        let _ = writeln!(out, "    fun test{method_name}() = runBlocking {{");
    } else {
        let _ = writeln!(out, "    fun test{method_name}() {{");
    }
    let _ = writeln!(out, "        // {description}");

    // Emit ObjectMapper deserialization bindings for json_object args.
    // Object args use the configured `options_type`. Array args carrying
    // `element_type = BatchBytesItem | BatchFileItem` are emitted as inline
    // List<T> constructors below (build_args_and_setup) — no deser binding is
    // needed because the array is materialised directly in source.
    if needs_deser {
        for arg in args {
            if arg.arg_type != "json_object" {
                continue;
            }
            let val = super::resolve_field(&fixture.input, &arg.field);
            if val.is_null() {
                continue;
            }
            // Skip arrays that we materialise inline (batch items + primitive
            // lists like List<String>) rather than deserialising via Jackson.
            if val.is_array() && arg.element_type.is_some() {
                continue;
            }
            let Some(opts_type) = options_type else { continue };
            let normalized = super::transform_json_keys_for_language(val, "snake_case");
            let json_str = serde_json::to_string(&normalized).unwrap_or_default();
            let var_name = &arg.name;
            let _ = writeln!(
                out,
                "        val {var_name} = MAPPER.readValue(\"{}\", {opts_type}::class.java)",
                escape_kotlin(&json_str)
            );
        }
    }

    let (setup_lines, args_str) =
        build_args_and_setup(fixture, &fixture.input, args, class_name, options_type, &fixture.id);

    // When client_factory is set, emit client-object instantiation + instance method call.
    // The factory name is a function on the Kotlin facade object (e.g. `LiterLlm.createClient`)
    // that constructs the coroutine-friendly Kotlin client wrapper from the
    // raw apiKey + baseUrl pair the test owns.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        // Prefer system properties set by MockServerListener (which spawns the
        // mock-server in-process when MOCK_SERVER_URL isn't pre-set). The
        // per-fixture property holds the full URL; fall back to the base URL
        // (mockServerUrl or env var) with the /fixtures/<id> suffix appended.
        let mock_url_expr = format!(
            "System.getProperty(\"mockServer.{fixture_id}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") + \"/fixtures/{fixture_id}\")"
        );
        for line in &setup_lines {
            let _ = writeln!(out, "        {line}");
        }
        let _ = writeln!(
            out,
            "        val client = {class_name}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
        );
        if expects_error {
            let _ = writeln!(out, "        assertFailsWith<Exception> {{");
            let _ = writeln!(out, "            client.{function_name}({args_str})");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "        client.close()");
            let _ = writeln!(out, "    }}");
            return;
        }
        let _ = writeln!(out, "        val {result_var} = client.{function_name}({args_str})");
        if !collect_snippet.is_empty() {
            let _ = writeln!(out, "        {collect_snippet}");
        }
        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                class_name,
                field_resolver,
                result_is_simple,
                result_is_option,
                enum_fields,
                &e2e_config.fields_c_types,
                is_streaming,
                kotlin_android_style,
            );
        }
        let _ = writeln!(out, "        client.close()");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Flat-function call style (no client_factory).
    if expects_error {
        // Wrap setup + call in assertFailsWith so validation errors thrown
        // during engine creation are also caught (mirrors Java's assertThrows).
        let _ = writeln!(out, "        assertFailsWith<Exception> {{");
        for line in &setup_lines {
            let _ = writeln!(out, "            {line}");
        }
        let _ = writeln!(out, "            {class_name}.{function_name}({args_str})");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    let _ = writeln!(
        out,
        "        val {result_var} = {class_name}.{function_name}({args_str})"
    );

    if !collect_snippet.is_empty() {
        let _ = writeln!(out, "        {collect_snippet}");
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            class_name,
            field_resolver,
            result_is_simple,
            result_is_option,
            enum_fields,
            &e2e_config.fields_c_types,
            is_streaming,
            kotlin_android_style,
        );
    }

    let _ = writeln!(out, "    }}");
}

/// Build setup lines and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
fn build_args_and_setup(
    fixture: &Fixture,
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    class_name: &str,
    options_type: Option<&str>,
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                setup_lines.push(format!(
                    "val {} = System.getProperty(\"mockServer.{fixture_id}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\")",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "val {} = System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("val {} = {class_name}.{constructor_name}(null)", arg.name,));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let name = &arg.name;
                setup_lines.push(format!(
                    "val {name}Config = MAPPER.readValue(\"{}\", CrawlConfig::class.java)",
                    escape_kotlin(&json_str),
                ));
                setup_lines.push(format!(
                    "val {} = {class_name}.{constructor_name}({name}Config)",
                    arg.name,
                    name = name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        // Use resolve_field so field = "input" resolves to the whole fixture input.
        let val_resolved = super::resolve_field(input, &arg.field);
        let val: Option<&serde_json::Value> = if val_resolved.is_null() {
            None
        } else {
            Some(val_resolved)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: emit positional default so the
                // call has the right arity for the Java facade. For json_object
                // optional args with a configured options_type, construct an empty
                // default builder instead of passing raw null.
                if arg.arg_type == "json_object" {
                    if let Some(opts_type) = options_type {
                        parts.push(format!("{opts_type}.builder().build()"));
                    } else {
                        parts.push("null".to_string());
                    }
                } else {
                    parts.push("null".to_string());
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // Typed arrays carry `element_type`. Batch item arrays
                // (BatchBytesItem/BatchFileItem) need typed constructors; all
                // other typed lists (e.g. List<String>) are materialised as a
                // plain `listOf(...)` of the JSON literals.
                if arg.arg_type == "json_object" && v.is_array() {
                    if let Some(elem) = &arg.element_type {
                        if elem == "BatchBytesItem" || elem == "BatchFileItem" {
                            parts.push(emit_kotlin_batch_item_array(v, elem));
                            continue;
                        }
                        // Generic typed list — emit literal Kotlin `listOf(...)`.
                        let items: Vec<String> = v
                            .as_array()
                            .map(|arr| arr.iter().map(json_to_kotlin).collect())
                            .unwrap_or_default();
                        parts.push(format!("listOf({})", items.join(", ")));
                        continue;
                    }
                }
                // For json_object args with options_type, use the pre-deserialized variable.
                if arg.arg_type == "json_object" && options_type.is_some() {
                    parts.push(arg.name.clone());
                    continue;
                }
                // bytes args carry a relative file path (e.g. "docx/fake.docx") that the
                // e2e harness resolves against test_documents/. Read the file at runtime
                // instead of converting the path string to its UTF-8 bytes.
                if arg.arg_type == "bytes" {
                    let val = json_to_kotlin(v);
                    parts.push(format!(
                        "java.nio.file.Files.readAllBytes(java.nio.file.Path.of({val}))"
                    ));
                    continue;
                }
                // file_path args must be wrapped in java.nio.file.Path.of(),
                // since the Kotlin module re-exports the Java facade signatures
                // which take Path rather than String for file-path parameters.
                if arg.arg_type == "file_path" {
                    let val = json_to_kotlin(v);
                    parts.push(format!("java.nio.file.Path.of({val})"));
                    continue;
                }
                parts.push(json_to_kotlin(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

/// Emit a Kotlin `listOf(...)` expression of `BatchBytesItem` or
/// `BatchFileItem` constructors. Mirrors `emit_java_batch_item_array` so the
/// Kotlin tests build the same typed lists the Java facade expects.
fn emit_kotlin_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
    let Some(items) = arr.as_array() else {
        return "emptyList()".to_string();
    };
    let parts: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            match elem_type {
                "BatchBytesItem" => {
                    let mime_type = obj.get("mime_type").and_then(|v| v.as_str()).unwrap_or("text/plain");
                    let content_code = obj
                        .get("content")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            let bytes: Vec<String> =
                                arr.iter().filter_map(|v| v.as_u64().map(|n| format!("{n}"))).collect();
                            format!("byteArrayOf({})", bytes.join(", "))
                        })
                        .unwrap_or_else(|| "byteArrayOf()".to_string());
                    Some(format!("{elem_type}({content_code}, \"{mime_type}\", null)"))
                }
                "BatchFileItem" => {
                    let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    Some(format!("{elem_type}(java.nio.file.Paths.get(\"{path}\"), null)"))
                }
                _ => None,
            }
        })
        .collect();
    format!("listOf({})", parts.join(", "))
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_option: bool,
    enum_fields: &HashSet<String>,
    fields_c_types: &std::collections::HashMap<String, String>,
    is_streaming: bool,
    kotlin_android_style: bool,
) {
    // In streaming context, `usage` and `usage.*` fields must be read from the
    // last collected chunk, not from the stream iterator (which has no `usage()` method).
    // Route them through `StreamingFieldResolver::accessor("usage", ...)` + deep-tail
    // rendering, using `chunks.last().usage()` as the base expression.
    if is_streaming {
        if let Some(f) = &assertion.field {
            if f == "usage" || f.starts_with("usage.") {
                let base_expr =
                    crate::codegen::streaming_assertions::StreamingFieldResolver::accessor("usage", "kotlin", "chunks")
                        .unwrap_or_else(|| "(if (chunks.isEmpty()) null else chunks.last().usage())".to_string());

                // For a deep path like `usage.total_tokens`, render the tail `.total_tokens`
                // in a Kotlin-idiomatic style (safe-call + camelCase method).
                let expr = if let Some(tail) = f.strip_prefix("usage.") {
                    use heck::ToLowerCamelCase;
                    // Each segment in the tail is a field accessor using `?.` (nullable base).
                    tail.split('.')
                        .fold(base_expr, |acc, seg| format!("{acc}?.{}()", seg.to_lower_camel_case()))
                } else {
                    base_expr
                };

                // Determine if the field maps to a 64-bit C type requiring `L` suffix.
                let field_is_long = fields_c_types
                    .get(f.as_str())
                    .is_some_and(|t| matches!(t.as_str(), "uint64_t" | "int64_t"));

                let line = match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(expected) = &assertion.value {
                            let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
                                format!("{}L", expected)
                            } else {
                                json_to_kotlin(expected)
                            };
                            format!("        assertEquals({kotlin_val}, {expr}!!)\n")
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
                return;
            }
        }
    }

    // Streaming virtual fields resolve against the `chunks` collected-list variable.
    // Intercept before is_valid_for_result so they are never skipped.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && crate::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "kotlin", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr}.size >= {n}, \"expected >= {n} chunks\")\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!(
                                "        assertEquals({n}.toLong(), {expr}.size.toLong(), \"expected exactly {n} elements\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_kotlin(s);
                            format!("        assertEquals(\"{escaped}\", {expr})\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        assertEquals({b}, {expr})\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => {
                        format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        assertTrue({expr}.isEmpty(), \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        assertTrue({expr}, \"expected true\")\n")
                    }
                    "is_false" => {
                        format!("        assertFalse({expr}, \"expected false\")\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} > {n}, \"expected > {n}\")\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_kotlin(s);
                            format!(
                                "        assertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    _ => format!(
                        "        // streaming field '{f}': assertion type '{}' not rendered\n",
                        assertion.assertion_type
                    ),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    // Raw field accessor — may end with nullable type if field is optional.
    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "kotlin", result_var),
            _ => result_var.to_string(),
        }
    };

    // Whether the accessor may return a nullable type in Kotlin. This is true
    // when the leaf field OR any intermediate segment in the path is optional
    // (the `?.` safe-call propagates null through the whole chain).
    //
    // Additionally, if the generated accessor expression itself contains `?.`
    // then the return type is `T?` regardless of what the path-resolver says —
    // sticky nullability means any `?.` in the chain makes the whole expression
    // nullable. This handles cases like `toolCalls()?.first()?.function()?.name()`
    // where the `is_optional` prefix lookup misses due to index notation mismatch.
    let field_is_optional = !result_is_simple
        && (field_expr.contains("?.")
            || assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                if field_resolver.has_map_access(f) {
                    return false;
                }
                // Check the leaf field itself.
                if field_resolver.is_optional(resolved) {
                    return true;
                }
                // Also check every prefix segment: if any intermediate field is
                // optional the ?.  chain propagates null to the final result.
                let mut prefix = String::new();
                for part in resolved.split('.') {
                    // Strip array notation for the lookup key.
                    let key = part.split('[').next().unwrap_or(part);
                    if !prefix.is_empty() {
                        prefix.push('.');
                    }
                    prefix.push_str(key);
                    if field_resolver.is_optional(&prefix) {
                        return true;
                    }
                }
                false
            }));

    // String-context expression: append .orEmpty() for nullable string fields so
    // string operations (contains, trim) don't require a safe-call chain.
    // Note: this is only sound when the leaf type is `String?`. For enum-typed
    // optional fields (`T?` where `T` is an enum class), `.orEmpty()` is undefined;
    // the enum branch below handles those by going through `?.getValue()` first.
    let string_field_expr = if field_is_optional {
        format!("{field_expr}.orEmpty()")
    } else {
        field_expr.clone()
    };

    // Non-null expression: use !! to assert presence for numeric comparisons where
    // the fixture guarantees the value is non-null.
    let nonnull_field_expr = if field_is_optional {
        format!("{field_expr}!!")
    } else {
        field_expr.clone()
    };

    // For enum fields, use .getValue() to get the string value. When the enum
    // field (or any intermediate segment in its path) is optional, use a safe
    // call before `.getValue()` and then `.orEmpty()` to coerce `String?` back
    // to `String` — this matches the Java codegen's
    // `Optional.ofNullable(...).map(v -> v.getValue()).orElse("")` pattern.
    let string_expr = match (field_is_enum, field_is_optional) {
        (true, true) => format!("{field_expr}?.getValue().orEmpty()"),
        (true, false) => format!("{field_expr}.getValue()"),
        (false, _) => string_field_expr.clone(),
    };

    // Determine if this assertion field maps to a 64-bit C type (uint64_t / int64_t),
    // which corresponds to Kotlin `Long`. When true, integer literals must be suffixed
    // with `L` to avoid a type mismatch between Kotlin `Int` and `Long`.
    let field_is_long = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        matches!(
            fields_c_types.get(resolved).map(String::as_str),
            Some("uint64_t") | Some("int64_t")
        )
    });

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                // Suffix integer literals with `L` when the target field is a Java `long`
                // (uint64_t / int64_t in C FFI terms). Without the suffix, Kotlin infers
                // the literal as `Int`, causing a type mismatch with `Long` at runtime.
                let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
                    format!("{}L", expected)
                } else {
                    json_to_kotlin(expected)
                };
                if expected.is_string() {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {string_expr}.trim())");
                } else {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {nonnull_field_expr})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let kotlin_val = json_to_kotlin(val);
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertFalse({string_expr}.contains({kotlin_val}), \"expected NOT to contain: \" + {kotlin_val})"
                );
            }
        }
        "not_empty" => {
            // For optional fields, the field type may be a non-String object
            // (e.g. DocumentStructure) for which `.orEmpty()` is undefined. A
            // null-check is the safe primitive: it works for any reference type
            // and matches the Java codegen's `Optional.ofNullable(...).isEmpty()`.
            // When the bare result is `T?` (result_is_option) the same null-check
            // applies, because `.isEmpty()` is undefined on arbitrary nullable types.
            // The JVM Kotlin e2e tests call the Java facade class which returns
            // `java.util.Optional<T>` for option results — use `.isPresent` rather
            // than `!= null` so the assertion semantics match the JVM return type.
            // The kotlin-android wrapper unwraps `Optional<T>` to Kotlin's `T?`
            // at the boundary, so its bare-option result is a nullable reference
            // and must use `!= null` instead.
            let bare_result_is_option =
                result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
            if bare_result_is_option && !kotlin_android_style {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr}.isPresent, \"expected non-empty value\")"
                );
            } else if bare_result_is_option || field_is_optional {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} != null, \"expected non-empty value\")"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        assertFalse({string_field_expr}.isEmpty(), \"expected non-empty value\")"
                );
            }
        }
        "is_empty" => {
            let bare_result_is_option =
                result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
            if bare_result_is_option && !kotlin_android_style {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr}.isEmpty, \"expected empty value\")"
                );
            } else if bare_result_is_option || field_is_optional {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} == null, \"expected empty value\")"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        assertTrue({string_field_expr}.isEmpty(), \"expected empty value\")"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let kotlin_val = json_to_kotlin(v);
                        format!("{string_expr}.contains({kotlin_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        assertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} > {kotlin_val}, \"expected > {kotlin_val}\")"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} < {kotlin_val}, \"expected < {kotlin_val}\")"
                );
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} >= {kotlin_val}, \"expected >= {kotlin_val}\")"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} <= {kotlin_val}, \"expected <= {kotlin_val}\")"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.startsWith({kotlin_val}), \"expected to start with: \" + {kotlin_val})"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.endsWith({kotlin_val}), \"expected to end with: \" + {kotlin_val})"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_field_expr}.length >= {n}, \"expected length >= {n}\")"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_field_expr}.length <= {n}, \"expected length <= {n}\")"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({nonnull_field_expr}.size >= {n}, \"expected at least {n} elements\")"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertEquals({n}, {nonnull_field_expr}.size, \"expected exactly {n} elements\")"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "        assertTrue({field_expr}, \"expected true\")");
        }
        "is_false" => {
            let _ = writeln!(out, "        assertFalse({field_expr}, \"expected false\")");
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue(Regex({kotlin_val}).containsMatchIn({string_expr}), \"expected value to match regex: \" + {kotlin_val})"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            // Placeholder: Kotlin support for method_result would need tree-sitter integration.
            let _ = writeln!(
                out,
                "        // method_result assertions not yet implemented for Kotlin"
            );
        }
        other => {
            panic!("Kotlin e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Kotlin literal string.
fn json_to_kotlin(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_kotlin(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                // Kotlin Double literals use no suffix (or `.0` if integer-shaped).
                // `0.9d` would parse as identifier `d` following a malformed literal.
                let s = n.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    s
                } else {
                    format!("{s}.0")
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_kotlin).collect();
            format!("listOf({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_kotlin(&json_str))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_resolver_for_finish_reason() -> FieldResolver {
        // Resolver for `choices[0].finish_reason` where:
        //   - `choices` is a registered array field (default index 0)
        //   - `choices.finish_reason` is optional (`@Nullable`)
        let mut optional = HashSet::new();
        optional.insert("choices.finish_reason".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("choices".to_string());
        FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
    }

    /// Regression: enum-typed optional fields must route through `?.getValue()`
    /// before falling back via `.orEmpty()`. Emitting `.orEmpty().getValue()`
    /// is invalid Kotlin because `T?.orEmpty()` is only defined for `String?`.
    #[test]
    fn assertion_enum_optional_uses_safe_get_value_then_or_empty() {
        let resolver = make_resolver_for_finish_reason();
        let mut enum_fields = HashSet::new();
        enum_fields.insert("choices.finish_reason".to_string());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("choices.finish_reason".to_string()),
            value: Some(serde_json::Value::String("stop".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "",
            &resolver,
            false,
            false,
            &enum_fields,
            &HashMap::new(),
            false,
            false,
        );
        assert!(
            out.contains("result.choices().first().finishReason()?.getValue().orEmpty().trim()"),
            "expected enum-optional safe-call pattern, got: {out}"
        );
        assert!(
            !out.contains(".finishReason().orEmpty().getValue()"),
            "must not emit .orEmpty().getValue() on a nullable enum: {out}"
        );
    }

    /// Non-optional enum field should call `.getValue()` directly without
    /// safe-call or fallback (no need to handle null).
    #[test]
    fn assertion_enum_non_optional_uses_plain_get_value() {
        let mut arrays = HashSet::new();
        arrays.insert("choices".to_string());
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &arrays,
            &HashSet::new(),
        );
        let mut enum_fields = HashSet::new();
        enum_fields.insert("choices.finish_reason".to_string());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("choices.finish_reason".to_string()),
            value: Some(serde_json::Value::String("stop".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "",
            &resolver,
            false,
            false,
            &enum_fields,
            &HashMap::new(),
            false,
            false,
        );
        assert!(
            out.contains("result.choices().first().finishReason().getValue().trim()"),
            "expected plain .getValue() for non-optional enum, got: {out}"
        );
    }

    /// Regression: per-call `enum_fields` overrides (e.g. `status = "BatchStatus"`) must be
    /// merged into the effective enum-field set before rendering assertions.  Previously the
    /// kotlin codegen only consulted the global `fields_enum` set, so `status` on `BatchObject`
    /// was treated as a plain `String` and `.trim()` was emitted directly instead of
    /// `.getValue().trim()`, causing a Kotlin compile error ("BatchStatus has no method trim").
    #[test]
    fn per_call_enum_field_override_routes_through_get_value() {
        // Simulate `status` field on a non-optional result with no global enum registration.
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        // `status` is NOT in the global enum_fields set...
        let global_enum_fields: HashSet<String> = HashSet::new();
        // ...but a per-call override registers it.
        let mut per_call_enum_fields: HashSet<String> = global_enum_fields.clone();
        per_call_enum_fields.insert("status".to_string());

        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("status".to_string()),
            value: Some(serde_json::Value::String("validating".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        };

        // Without the merge (global only): must NOT emit .getValue()
        let mut out_no_merge = String::new();
        render_assertion(
            &mut out_no_merge,
            &assertion,
            "result",
            "",
            &resolver,
            false,
            false,
            &global_enum_fields,
            &HashMap::new(),
            false,
            false,
        );
        assert!(
            !out_no_merge.contains(".getValue()"),
            "global-only set must not emit .getValue() for unregistered status: {out_no_merge}"
        );

        // With the merge (per-call included): must emit .getValue()
        let mut out_merged = String::new();
        render_assertion(
            &mut out_merged,
            &assertion,
            "result",
            "",
            &resolver,
            false,
            false,
            &per_call_enum_fields,
            &HashMap::new(),
            false,
            false,
        );
        assert!(
            out_merged.contains(".getValue()"),
            "merged per-call set must emit .getValue() for status: {out_merged}"
        );
    }

    /// Auto-detection: fields whose Rust type is `Named(T)` where `T` is NOT a
    /// known struct should be treated as enum-typed without any explicit per-call
    /// `enum_fields` override. The `type_enum_fields` map (built in `generate()`)
    /// pre-computes these sets so `render_test_method` can merge them.
    #[test]
    fn auto_detected_enum_fields_from_type_defs_route_through_get_value() {
        use alef_core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};

        // Simulate a `BatchObject` type with `status: BatchStatus` (Named, not a struct).
        let batch_object_def = TypeDef {
            name: "BatchObject".to_string(),
            rust_path: "liter_llm::BatchObject".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "id".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                },
                FieldDef {
                    name: "status".to_string(),
                    ty: TypeRef::Named("BatchStatus".to_string()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
        };

        // `BatchObject` is the only struct — `BatchStatus` is not in struct_names.
        let type_defs = [batch_object_def];
        let struct_names: HashSet<&str> = type_defs.iter().map(|td| td.name.as_str()).collect();

        // Verify is_enum_typed correctly identifies `status` as enum-typed.
        let status_ty = TypeRef::Named("BatchStatus".to_string());
        assert!(
            is_enum_typed(&status_ty, &struct_names),
            "BatchStatus (not a known struct) should be detected as enum-typed"
        );
        let id_ty = TypeRef::String;
        assert!(
            !is_enum_typed(&id_ty, &struct_names),
            "String field should NOT be detected as enum-typed"
        );

        // Verify the type_enum_fields map is built correctly.
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

        let batch_enum_fields = type_enum_fields
            .get("BatchObject")
            .expect("BatchObject should have enum fields");
        assert!(
            batch_enum_fields.contains("status"),
            "BatchObject.status should be auto-detected as enum-typed, got: {batch_enum_fields:?}"
        );
        assert!(
            !batch_enum_fields.contains("id"),
            "BatchObject.id (String) must not be in enum fields"
        );

        // Verify render_assertion produces `.getValue()` when `status` is in enum_fields.
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("status".to_string()),
            value: Some(serde_json::Value::String("validating".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "",
            &resolver,
            false,
            false,
            batch_enum_fields,
            &HashMap::new(),
            false,
            false,
        );
        assert!(
            out.contains(".getValue()"),
            "auto-detected enum field must route through .getValue(), got: {out}"
        );
    }
}
