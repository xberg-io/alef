use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions::{maven, toolchain};
use std::fmt::Write as FmtWrite;

/// Render build.gradle.kts for the kotlin e2e project.
pub(crate) fn render_build_gradle(
    pkg_name: &str,
    kotlin_pkg_id: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    _has_http_fixtures: bool,
) -> String {
    let dep_block = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Registry mode: maven central with group:artifact:version.
            //
            // `pkg_name` is the published artifactId only (e.g. `sample_project-kotlin`);
            // the group is `kotlin_pkg_id` (the `[kotlin] package`, e.g.
            // `dev.sample_project`). Guard against a `pkg_name` that already embeds the
            // group (e.g. `dev.sample_project:sample_project-kotlin`) so the group is never
            // prepended twice — otherwise gradle resolves a non-existent
            // `dev.sample_project:dev.sample_project:sample_project-kotlin` coordinate.
            let coordinate = if pkg_name.starts_with(&format!("{kotlin_pkg_id}:")) {
                format!("{pkg_name}:{pkg_version}")
            } else {
                format!("{kotlin_pkg_id}:{pkg_name}:{pkg_version}")
            };
            format!(r#"    testImplementation("{coordinate}")"#)
        }
        crate::e2e::config::DependencyMode::Local => {
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
    let jvm_target = toolchain::KOTLIN_JVM_TARGET;
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
    testImplementation("com.fasterxml.jackson.core:jackson-databind:{jackson}")
    testImplementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")
    testImplementation(kotlin("test"))
}}

tasks.test {{
    useJUnitPlatform()
    val libPath = System.getProperty("native.lib.path") ?: "${{rootDir}}/../../target/release"
    systemProperty("java.library.path", libPath)
    systemProperty("jna.library.path", libPath)
    // Panama FFI bindings are compiled with --enable-preview against the
    // java.lang.foreign API, so the forked test worker must enable preview +
    // native access — otherwise the worker JVM aborts before JUnit starts and
    // Gradle reports a misleading "Gradle Test Executor N ... not in started or
    // detached state". Mirrors the Maven surefire argLine.
    jvmArgs("--enable-preview", "--enable-native-access=ALL-UNNAMED")
    // Resolve fixture paths (e.g. "docx/fake.docx") against test_documents/ when
    // the consumer ships such fixtures. Guard on existence: Gradle test workers
    // fail to fork if workingDir points at a directory that does not exist.
    val testDocuments = file("${{rootDir}}/../../test_documents")
    if (testDocuments.isDirectory) {{
        workingDir = testDocuments
    }}
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
            // Even when MOCK_SERVER_URL is preset (alef test-apps runner mode),
            // the runner also exports MOCK_SERVERS as a JSON env var of
            // fixture_id -> url. Translate it into mockServer.<fixture_id> system
            // properties so tests that target a dedicated per-fixture server
            // (e.g. asset-download tests) resolve it instead of falling back to
            // the shared server. Mirrors the spawn path below and the Java listener.
            val presetServers = System.getenv("MOCK_SERVERS")
            if (!presetServers.isNullOrEmpty()) {{
                System.setProperty("mockServers", presetServers)
                val p = Pattern.compile(""""([^"]+)":"([^"]+)"""")
                val matcher = p.matcher(presetServers)
                while (matcher.find()) {{
                    System.setProperty("mockServer.${{matcher.group(1)}}", matcher.group(2))
                }}
            }}
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

/// Render SutServerSetup.kt with JUnit 5 @BeforeAll fixture to set SUT_URL.
pub(super) fn render_sut_server_setup_kt(kotlin_pkg_id: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    let mut out = String::new();
    let _ = writeln!(out, "{}", header);
    let _ = writeln!(out, "package {}.e2e", kotlin_pkg_id);
    let _ = writeln!(out);
    let _ = writeln!(out, "import org.junit.jupiter.api.BeforeAll");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * JUnit 5 setup that ensures SUT_URL is set before tests run.");
    let _ = writeln!(out, " * Defaults to http://127.0.0.1:8007 if not already set.");
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "class SutServerSetup {{");
    let _ = writeln!(out, "    companion object {{");
    let _ = writeln!(out, "        @BeforeAll");
    let _ = writeln!(out, "        @JvmStatic");
    let _ = writeln!(out, "        fun setupSutServer() {{");
    let _ = writeln!(
        out,
        "            val existing = System.getenv(\"SUT_URL\") ?: System.getProperty(\"SUT_URL\")"
    );
    let _ = writeln!(out, "            val url = if (!existing.isNullOrEmpty()) {{");
    let _ = writeln!(out, "                existing");
    let _ = writeln!(out, "            }} else {{");
    let _ = writeln!(out, "                \"http://127.0.0.1:8007\"");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            System.setProperty(\"SUT_URL\", url)");
    let _ = writeln!(out, "            println(\"Tests will use SUT at: $url\")");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    out
}
