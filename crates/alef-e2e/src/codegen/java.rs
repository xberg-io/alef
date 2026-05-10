//! Java e2e test generator using JUnit 5.
//!
//! Generates `e2e/java/pom.xml` and `src/test/java/dev/kreuzberg/e2e/{Category}Test.java`
//! files from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_java, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Java e2e code generator.
pub struct JavaCodegen;

impl E2eCodegen for JavaCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[alef_core::ir::TypeDef],
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
        let java_pkg = e2e_config.resolve_package("java");
        let pkg_name = java_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.clone());

        // Resolve Java package info for the dependency.
        let java_group_id = config.java_group_id();
        let binding_pkg = config.java_package();
        let pkg_version = config.resolved_version().unwrap_or_else(|| "0.1.0".to_string());

        // Generate pom.xml.
        files.push(GeneratedFile {
            path: output_base.join("pom.xml"),
            content: render_pom_xml(
                &pkg_name,
                &java_group_id,
                &pkg_version,
                e2e_config.dep_mode,
                &e2e_config.test_documents_relative_from(0),
            ),
            generated_header: false,
        });

        // Detect whether any fixture needs the mock-server (HTTP fixtures or
        // fixtures with a `mock_response`). When present, emit a
        // JUnit Platform LauncherSessionListener that spawns the mock-server
        // before any test runs and a META-INF/services SPI manifest registering
        // it. Without this, every fixture-bound test failed with
        // `LiterLlmRsException: error sending request for url` because
        // `System.getenv("MOCK_SERVER_URL")` was null.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Generate test files per category. Path mirrors the configured Java
        // package — `dev.myorg` becomes `dev/myorg`, etc. — so the package
        // declaration in each test file matches its filesystem location.
        let mut test_base = output_base.join("src").join("test").join("java");
        for segment in java_group_id.split('.') {
            test_base = test_base.join(segment);
        }
        let test_base = test_base.join("e2e");

        if needs_mock_server {
            files.push(GeneratedFile {
                path: test_base.join("MockServerListener.java"),
                content: render_mock_server_listener(&java_group_id),
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
                content: format!("{java_group_id}.e2e.MockServerListener\n"),
                generated_header: false,
            });
        }

        // Resolve options_type from override.
        let options_type = overrides.and_then(|o| o.options_type.clone());

        // Resolve enum_fields and nested_types from Java override config.
        static EMPTY_ENUM_FIELDS: std::sync::LazyLock<std::collections::HashMap<String, String>> =
            std::sync::LazyLock::new(std::collections::HashMap::new);
        let _enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&EMPTY_ENUM_FIELDS);

        // Build effective nested_types by merging defaults with configured overrides.
        let mut effective_nested_types = default_java_nested_types();
        if let Some(overrides_map) = overrides.map(|o| &o.nested_types) {
            effective_nested_types.extend(overrides_map.clone());
        }

        // Resolve nested_types_optional from override (defaults to true for backward compatibility).
        let nested_types_optional = overrides.map(|o| o.nested_types_optional).unwrap_or(true);

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &std::collections::HashSet::new(),
        );

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_file_name = format!("{}Test.java", sanitize_filename(&group.category).to_upper_camel_case());
            let content = render_test_file(
                &group.category,
                &active,
                &class_name,
                &function_name,
                &java_group_id,
                &binding_pkg,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
                e2e_config,
                &effective_nested_types,
                nested_types_optional,
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
        "java"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_pom_xml(
    pkg_name: &str,
    java_group_id: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
    test_documents_path: &str,
) -> String {
    // pkg_name may be in "groupId:artifactId" Maven format; split accordingly.
    let (dep_group_id, dep_artifact_id) = if let Some((g, a)) = pkg_name.split_once(':') {
        (g, a)
    } else {
        (java_group_id, pkg_name)
    };
    let artifact_id = format!("{dep_artifact_id}-e2e-java");
    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!(
                r#"        <dependency>
            <groupId>{dep_group_id}</groupId>
            <artifactId>{dep_artifact_id}</artifactId>
            <version>{pkg_version}</version>
        </dependency>"#
            )
        }
        crate::config::DependencyMode::Local => {
            format!(
                r#"        <dependency>
            <groupId>{dep_group_id}</groupId>
            <artifactId>{dep_artifact_id}</artifactId>
            <version>{pkg_version}</version>
            <scope>system</scope>
            <systemPath>${{project.basedir}}/../../packages/java/target/{dep_artifact_id}-{pkg_version}.jar</systemPath>
        </dependency>"#
            )
        }
    };
    crate::template_env::render(
        "java/pom.xml.jinja",
        minijinja::context! {
            artifact_id => artifact_id,
            java_group_id => java_group_id,
            dep_block => dep_block,
            junit_version => tv::maven::JUNIT,
            jackson_version => tv::maven::JACKSON_E2E,
            build_helper_version => tv::maven::BUILD_HELPER_MAVEN_PLUGIN,
            maven_surefire_version => tv::maven::MAVEN_SUREFIRE_PLUGIN_E2E,
            test_documents_path => test_documents_path,
        },
    )
}

/// Render the JUnit Platform LauncherSessionListener that spawns the
/// mock-server binary once per launcher session and tears it down on close.
///
/// Mirrors the Ruby `spec_helper.rb` and Python `conftest.py` patterns. The
/// URL is exposed as a JVM system property `mockServerUrl`; generated test
/// bodies prefer it over the `MOCK_SERVER_URL` env var so external overrides
/// (e.g. CI exporting MOCK_SERVER_URL) still work without rerouting through
/// JNI's lack of `setenv`.
fn render_mock_server_listener(java_group_id: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("package {java_group_id}.e2e;\n\n"));
    out.push_str("import java.io.BufferedReader;\n");
    out.push_str("import java.io.File;\n");
    out.push_str("import java.io.IOException;\n");
    out.push_str("import java.io.InputStreamReader;\n");
    out.push_str("import java.nio.charset.StandardCharsets;\n");
    out.push_str("import java.nio.file.Path;\n");
    out.push_str("import java.nio.file.Paths;\n");
    out.push_str("import org.junit.platform.launcher.LauncherSession;\n");
    out.push_str("import org.junit.platform.launcher.LauncherSessionListener;\n");
    out.push('\n');
    out.push_str("/**\n");
    out.push_str(" * Spawns the mock-server binary once per JUnit launcher session and\n");
    out.push_str(" * exposes its URL as the `mockServerUrl` system property. Generated\n");
    out.push_str(" * test bodies read the property (with `MOCK_SERVER_URL` env-var\n");
    out.push_str(" * fallback) so tests can run via plain `mvn test` without any external\n");
    out.push_str(" * mock-server orchestration. Mirrors the Ruby spec_helper / Python\n");
    out.push_str(" * conftest spawn pattern. Honors a pre-set MOCK_SERVER_URL by\n");
    out.push_str(" * skipping the spawn entirely.\n");
    out.push_str(" */\n");
    out.push_str("public class MockServerListener implements LauncherSessionListener {\n");
    out.push_str("    private Process mockServer;\n");
    out.push('\n');
    out.push_str("    @Override\n");
    out.push_str("    public void launcherSessionOpened(LauncherSession session) {\n");
    out.push_str("        String preset = System.getenv(\"MOCK_SERVER_URL\");\n");
    out.push_str("        if (preset != null && !preset.isEmpty()) {\n");
    out.push_str("            System.setProperty(\"mockServerUrl\", preset);\n");
    out.push_str("            return;\n");
    out.push_str("        }\n");
    out.push_str("        Path repoRoot = locateRepoRoot();\n");
    out.push_str("        if (repoRoot == null) {\n");
    out.push_str("            throw new IllegalStateException(\"MockServerListener: could not locate repo root (looked for fixtures/ in ancestors of \" + System.getProperty(\"user.dir\") + \")\");\n");
    out.push_str("        }\n");
    out.push_str("        String binName = System.getProperty(\"os.name\", \"\").toLowerCase().contains(\"win\") ? \"mock-server.exe\" : \"mock-server\";\n");
    out.push_str("        File bin = repoRoot.resolve(\"e2e\").resolve(\"rust\").resolve(\"target\").resolve(\"release\").resolve(binName).toFile();\n");
    out.push_str("        File fixturesDir = repoRoot.resolve(\"fixtures\").toFile();\n");
    out.push_str("        if (!bin.exists()) {\n");
    out.push_str("            throw new IllegalStateException(\"MockServerListener: mock-server binary not found at \" + bin + \" — run: cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release\");\n");
    out.push_str("        }\n");
    out.push_str(
        "        ProcessBuilder pb = new ProcessBuilder(bin.getAbsolutePath(), fixturesDir.getAbsolutePath())\n",
    );
    out.push_str("            .redirectErrorStream(false);\n");
    out.push_str("        try {\n");
    out.push_str("            mockServer = pb.start();\n");
    out.push_str("        } catch (IOException e) {\n");
    out.push_str(
        "            throw new IllegalStateException(\"MockServerListener: failed to start mock-server\", e);\n",
    );
    out.push_str("        }\n");
    out.push_str("        // Read until we see the MOCK_SERVER_URL=... line. Cap the loop so a\n");
    out.push_str("        // misbehaving mock-server cannot block the launcher indefinitely.\n");
    out.push_str("        BufferedReader stdout = new BufferedReader(new InputStreamReader(mockServer.getInputStream(), StandardCharsets.UTF_8));\n");
    out.push_str("        String url = null;\n");
    out.push_str("        try {\n");
    out.push_str("            for (int i = 0; i < 16; i++) {\n");
    out.push_str("                String line = stdout.readLine();\n");
    out.push_str("                if (line == null) break;\n");
    out.push_str("                if (line.startsWith(\"MOCK_SERVER_URL=\")) {\n");
    out.push_str("                    url = line.substring(\"MOCK_SERVER_URL=\".length()).trim();\n");
    out.push_str("                    break;\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        } catch (IOException e) {\n");
    out.push_str("            mockServer.destroyForcibly();\n");
    out.push_str(
        "            throw new IllegalStateException(\"MockServerListener: failed to read mock-server stdout\", e);\n",
    );
    out.push_str("        }\n");
    out.push_str("        if (url == null || url.isEmpty()) {\n");
    out.push_str("            mockServer.destroyForcibly();\n");
    out.push_str("            throw new IllegalStateException(\"MockServerListener: mock-server did not emit MOCK_SERVER_URL\");\n");
    out.push_str("        }\n");
    out.push_str("        // TCP-readiness probe: ensure axum::serve is accepting before tests start.\n");
    out.push_str("        // The mock-server binds the TcpListener synchronously then prints the URL\n");
    out.push_str("        // before tokio::spawn(axum::serve(...)) is polled, so under Surefire\n");
    out.push_str("        // parallel mode tests can race startup. Poll-connect (max 5s, 50ms backoff)\n");
    out.push_str("        // until success.\n");
    out.push_str("        java.net.URI healthUri = java.net.URI.create(url);\n");
    out.push_str("        String host = healthUri.getHost();\n");
    out.push_str("        int port = healthUri.getPort();\n");
    out.push_str("        long deadline = System.nanoTime() + 5_000_000_000L;\n");
    out.push_str("        while (System.nanoTime() < deadline) {\n");
    out.push_str("            try (java.net.Socket s = new java.net.Socket()) {\n");
    out.push_str("                s.connect(new java.net.InetSocketAddress(host, port), 100);\n");
    out.push_str("                break;\n");
    out.push_str("            } catch (java.io.IOException ignored) {\n");
    out.push_str("                try { Thread.sleep(50); } catch (InterruptedException ie) { Thread.currentThread().interrupt(); break; }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        System.setProperty(\"mockServerUrl\", url);\n");
    out.push_str("        // Drain remaining stdout/stderr in daemon threads so a full pipe\n");
    out.push_str("        // does not block the child.\n");
    out.push_str("        Process server = mockServer;\n");
    out.push_str("        Thread drainOut = new Thread(() -> drain(stdout));\n");
    out.push_str("        drainOut.setDaemon(true);\n");
    out.push_str("        drainOut.start();\n");
    out.push_str("        Thread drainErr = new Thread(() -> drain(new BufferedReader(new InputStreamReader(server.getErrorStream(), StandardCharsets.UTF_8))));\n");
    out.push_str("        drainErr.setDaemon(true);\n");
    out.push_str("        drainErr.start();\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    @Override\n");
    out.push_str("    public void launcherSessionClosed(LauncherSession session) {\n");
    out.push_str("        if (mockServer == null) return;\n");
    out.push_str("        try { mockServer.getOutputStream().close(); } catch (IOException ignored) {}\n");
    out.push_str("        try {\n");
    out.push_str("            if (!mockServer.waitFor(2, java.util.concurrent.TimeUnit.SECONDS)) {\n");
    out.push_str("                mockServer.destroyForcibly();\n");
    out.push_str("            }\n");
    out.push_str("        } catch (InterruptedException ignored) {\n");
    out.push_str("            Thread.currentThread().interrupt();\n");
    out.push_str("            mockServer.destroyForcibly();\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    private static Path locateRepoRoot() {\n");
    out.push_str("        Path dir = Paths.get(\"\").toAbsolutePath();\n");
    out.push_str("        while (dir != null) {\n");
    out.push_str("            if (dir.resolve(\"fixtures\").toFile().isDirectory()\n");
    out.push_str("                && dir.resolve(\"e2e\").toFile().isDirectory()) {\n");
    out.push_str("                return dir;\n");
    out.push_str("            }\n");
    out.push_str("            dir = dir.getParent();\n");
    out.push_str("        }\n");
    out.push_str("        return null;\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    private static void drain(BufferedReader reader) {\n");
    out.push_str("        try {\n");
    out.push_str("            char[] buf = new char[1024];\n");
    out.push_str("            while (reader.read(buf) >= 0) { /* drain */ }\n");
    out.push_str("        } catch (IOException ignored) {}\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    java_group_id: &str,
    binding_pkg: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &std::collections::HashSet<String>,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls.  Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    // Check if any fixture (with its resolved call) will emit MAPPER usage.
    let lang_for_om = "java";
    let needs_object_mapper_for_handle = fixtures.iter().any(|f| {
        args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });
    // HTTP fixtures always need ObjectMapper for JSON body comparison.
    let has_http_fixtures = fixtures.iter().any(|f| f.http.is_some());
    let needs_object_mapper = needs_object_mapper_for_handle || has_http_fixtures;

    // Collect all options_type values used (class-level + per-fixture call overrides).
    let mut all_options_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(t) = options_type {
        all_options_types.insert(t.to_string());
    }
    for f in fixtures.iter() {
        let call_cfg = e2e_config.resolve_call(f.call.as_deref());
        if let Some(ov) = call_cfg.overrides.get(lang_for_om) {
            if let Some(t) = &ov.options_type {
                all_options_types.insert(t.clone());
            }
        }
        // Auto-fallback: when the Java override does not declare an options_type
        // but another non-prefixed binding (csharp/c/go/php/python) does, mirror
        // that name into the import set so the auto-emitted `Type.fromJson(json)`
        // expression compiles. The Java POJO class name matches the Rust source
        // type name for these backends.
        let java_has_type = call_cfg
            .overrides
            .get(lang_for_om)
            .and_then(|o| o.options_type.as_deref())
            .is_some();
        if !java_has_type {
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = call_cfg.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        all_options_types.insert(t.clone());
                        break;
                    }
                }
            }
        }
        // Detect batch item types used in this fixture
        for arg in &call_cfg.args {
            if let Some(elem_type) = &arg.element_type {
                if elem_type == "BatchBytesItem" || elem_type == "BatchFileItem" {
                    all_options_types.insert(elem_type.clone());
                }
            }
        }
    }

    // Collect nested config types actually referenced in fixture builder expressions.
    // Note: enum types don't need explicit imports since they're in the same package.
    let mut nested_types_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let call_cfg = e2e_config.resolve_call(f.call.as_deref());
        for arg in &call_cfg.args {
            if arg.arg_type == "json_object" {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                if let Some(val) = f.input.get(field) {
                    if !val.is_null() && !val.is_array() {
                        if let Some(obj) = val.as_object() {
                            collect_nested_type_names(obj, nested_types, &mut nested_types_used);
                        }
                    }
                }
            }
        }
    }

    // Effective binding package for FQN imports of binding types
    // (ChatCompletionRequest, etc.). Prefer the explicit `[crates.java] package`
    // wired in via `binding_pkg`; fall back to the package derived from a
    // fully-qualified `class_name` when present.
    let binding_pkg_for_imports: String = if !binding_pkg.is_empty() {
        binding_pkg.to_string()
    } else if !import_path.is_empty() {
        import_path
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Build imports list
    let mut imports: Vec<String> = Vec::new();
    imports.push("import org.junit.jupiter.api.Test;".to_string());
    imports.push("import static org.junit.jupiter.api.Assertions.*;".to_string());

    // Import the test entry-point class itself when it is fully-qualified or
    // when we know the binding package — emit the FQN so javac resolves it.
    if !import_path.is_empty() {
        imports.push(format!("import {import_path};"));
    } else if !binding_pkg_for_imports.is_empty() && !class_name.is_empty() {
        imports.push(format!("import {binding_pkg_for_imports}.{class_name};"));
    }

    if needs_object_mapper {
        imports.push("import com.fasterxml.jackson.databind.ObjectMapper;".to_string());
        imports.push("import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;".to_string());
    }

    // Import all options types used across fixtures (for builder expressions and MAPPER).
    if !all_options_types.is_empty() {
        for opts_type in &all_options_types {
            let qualified = if binding_pkg_for_imports.is_empty() {
                opts_type.clone()
            } else {
                format!("{binding_pkg_for_imports}.{opts_type}")
            };
            imports.push(format!("import {qualified};"));
        }
    }

    // Import nested options types
    if !nested_types_used.is_empty() && !binding_pkg_for_imports.is_empty() {
        for type_name in &nested_types_used {
            imports.push(format!("import {binding_pkg_for_imports}.{type_name};"));
        }
    }

    // Import CrawlConfig when handle args need JSON deserialization.
    if needs_object_mapper_for_handle && !binding_pkg_for_imports.is_empty() {
        imports.push(format!("import {binding_pkg_for_imports}.CrawlConfig;"));
    }

    // Import visitor types when any fixture uses visitor callbacks.
    let has_visitor_fixtures = fixtures.iter().any(|f| f.visitor.is_some());
    if has_visitor_fixtures && !binding_pkg_for_imports.is_empty() {
        imports.push(format!("import {binding_pkg_for_imports}.Visitor;"));
        imports.push(format!("import {binding_pkg_for_imports}.NodeContext;"));
        imports.push(format!("import {binding_pkg_for_imports}.VisitResult;"));
    }

    // Import Optional when using builder expressions with optional fields
    if !all_options_types.is_empty() {
        imports.push("import java.util.Optional;".to_string());
    }

    // Render all test methods
    let mut fixtures_body = String::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_method(
            &mut fixtures_body,
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
            nested_types,
            nested_types_optional,
        );
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    // Render template
    crate::template_env::render(
        "java/test_file.jinja",
        minijinja::context! {
            header => header,
            java_group_id => java_group_id,
            test_class_name => test_class_name,
            category => category,
            imports => imports,
            needs_object_mapper => needs_object_mapper,
            fixtures_body => fixtures_body,
        },
    )
}

// ---------------------------------------------------------------------------
// HTTP test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Thin renderer that emits JUnit 5 test methods targeting a mock server via
/// `java.net.http.HttpClient`. Satisfies [`client::TestClientRenderer`] so the
/// shared [`client::http_call::render_http_test`] driver drives the call sequence.
struct JavaTestClientRenderer;

impl client::TestClientRenderer for JavaTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "java"
    }

    /// Convert a fixture id to the UpperCamelCase suffix appended to `test`.
    ///
    /// The emitted method name is `test{fn_name}`, matching the pre-existing shape.
    fn sanitize_test_name(&self, id: &str) -> String {
        id.to_upper_camel_case()
    }

    /// Emit `@Test void test{fn_name}() throws Exception {`.
    ///
    /// When `skip_reason` is `Some`, the body is a single
    /// `Assumptions.assumeTrue(false, ...)` call and `render_test_close` closes
    /// the brace symmetrically.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_reason = skip_reason.map(escape_java);
        let rendered = crate::template_env::render(
            "java/http_test_open.jinja",
            minijinja::context! {
                fn_name => fn_name,
                description => description,
                skip_reason => escaped_reason,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit the closing `}` for a test method.
    fn render_test_close(&self, out: &mut String) {
        let rendered = crate::template_env::render("java/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit a `java.net.http.HttpClient` request to `baseUrl + path`.
    ///
    /// Binds the response to `response` (the `ctx.response_var`). Java's
    /// `HttpClient` disallows a fixed set of restricted headers; those are
    /// silently dropped so the test compiles.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        // Java's HttpClient throws IllegalArgumentException for these headers.
        const JAVA_RESTRICTED_HEADERS: &[&str] = &["connection", "content-length", "expect", "host", "upgrade"];

        let method = ctx.method.to_uppercase();

        // Build the path, appending query params when present.
        let path = if ctx.query_params.is_empty() {
            ctx.path.to_string()
        } else {
            let pairs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    format!("{}={}", k, escape_java(&val_str))
                })
                .collect();
            format!("{}?{}", ctx.path, pairs.join("&"))
        };

        let body_publisher = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_java(&json);
            format!("java.net.http.HttpRequest.BodyPublishers.ofString(\"{escaped}\")")
        } else {
            "java.net.http.HttpRequest.BodyPublishers.noBody()".to_string()
        };

        // Content-Type header — only when a body is present.
        let content_type = if ctx.body.is_some() {
            let ct = ctx.content_type.unwrap_or("application/json");
            // Only emit when not already in ctx.headers (avoid duplicate Content-Type).
            if !ctx.headers.keys().any(|k| k.to_lowercase() == "content-type") {
                Some(ct.to_string())
            } else {
                None
            }
        } else {
            None
        };

        // Build header lines — skip Java-restricted ones.
        let mut headers_lines: Vec<String> = Vec::new();
        for (name, value) in ctx.headers {
            if JAVA_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_java(name);
            let escaped_value = escape_java(value);
            headers_lines.push(format!(
                "builder = builder.header(\"{escaped_name}\", \"{escaped_value}\");"
            ));
        }

        // Cookies as a single `Cookie` header.
        let cookies_line = if !ctx.cookies.is_empty() {
            let cookie_str: Vec<String> = ctx.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_java(&cookie_str.join("; "));
            Some(format!("builder = builder.header(\"Cookie\", \"{cookie_header}\");"))
        } else {
            None
        };

        let rendered = crate::template_env::render(
            "java/http_request.jinja",
            minijinja::context! {
                method => method,
                path => path,
                body_publisher => body_publisher,
                content_type => content_type,
                headers_lines => headers_lines,
                cookies_line => cookies_line,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `assertEquals(status, response.statusCode(), ...)`.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let rendered = crate::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => status,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a header assertion using `response.headers().firstValue(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_java(name);
        let assertion_code = match expected {
            "<<present>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isPresent(), \"header {escaped_name} should be present\");"
                )
            }
            "<<absent>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isEmpty(), \"header {escaped_name} should be absent\");"
                )
            }
            "<<uuid>>" => {
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").matches(\"[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\"), \"header {escaped_name} should be a UUID\");"
                )
            }
            literal => {
                let escaped_value = escape_java(literal);
                format!(
                    "assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").contains(\"{escaped_value}\"), \"header {escaped_name} mismatch\");"
                )
            }
        };

        let mut headers = vec![std::collections::HashMap::new()];
        headers[0].insert("assertion_code", assertion_code);

        let rendered = crate::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => 0u16,
                headers => headers,
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a JSON body equality assertion using Jackson's `MAPPER.readTree`.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let body_assertion = match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_java(&json_str);
                format!(
                    "var bodyJson = MAPPER.readTree({response_var}.body());\n        var expectedJson = MAPPER.readTree(\"{escaped}\");\n        assertEquals(expectedJson, bodyJson, \"body mismatch\");"
                )
            }
            serde_json::Value::String(s) => {
                let escaped = escape_java(s);
                format!("assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");")
            }
            other => {
                let escaped = escape_java(&other.to_string());
                format!("assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");")
            }
        };

        let rendered = crate::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => body_assertion,
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit partial JSON body assertions: parse once, then assert each expected field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let mut partial_body: Vec<std::collections::HashMap<&str, String>> = Vec::new();
            for (key, val) in obj {
                let escaped_key = escape_java(key);
                let json_str = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = escape_java(&json_str);
                let assertion_code = format!(
                    "assertEquals(MAPPER.readTree(\"{escaped_val}\"), partialJson.get(\"{escaped_key}\"), \"body field '{escaped_key}' mismatch\");"
                );
                let mut entry = std::collections::HashMap::new();
                entry.insert("assertion_code", assertion_code);
                partial_body.push(entry);
            }

            let rendered = crate::template_env::render(
                "java/http_assertions.jinja",
                minijinja::context! {
                    response_var => response_var,
                    status_code => 0u16,
                    headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                    body_assertion => String::new(),
                    partial_body => partial_body,
                    validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
                },
            );
            out.push_str(&rendered);
        }
    }

    /// Emit validation-error assertions: parse the body and check each expected message.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[crate::fixture::ValidationErrorExpectation],
    ) {
        let mut validation_errors: Vec<std::collections::HashMap<&str, String>> = Vec::new();
        for err in errors {
            let escaped_msg = escape_java(&err.msg);
            let assertion_code = format!(
                "assertTrue(veBody.contains(\"{escaped_msg}\"), \"expected validation error message: {escaped_msg}\");"
            );
            let mut entry = std::collections::HashMap::new();
            entry.insert("assertion_code", assertion_code);
            validation_errors.push(entry);
        }

        let rendered = crate::template_env::render(
            "java/http_assertions.jinja",
            minijinja::context! {
                response_var => response_var,
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => validation_errors,
            },
        );
        out.push_str(&rendered);
    }
}

/// Render an HTTP server test method using `java.net.http.HttpClient` against
/// `MOCK_SERVER_URL`. Delegates to the shared
/// [`client::http_call::render_http_test`] driver via [`JavaTestClientRenderer`].
///
/// The one Java-specific pre-condition — HTTP 101 (WebSocket upgrade) causing an
/// `EOFException` in `HttpClient` — is handled here before delegating.
fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) causes Java's HttpClient to throw EOFException.
    // Emit an assumeTrue(false, ...) stub so the test is skipped rather than failing.
    if http.expected_response.status_code == 101 {
        let method_name = fixture.id.to_upper_camel_case();
        let description = &fixture.description;
        out.push_str(&crate::template_env::render(
            "java/http_test_skip_101.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
            },
        ));
        return;
    }

    client::http_call::render_http_test(out, &JavaTestClientRenderer, fixture);
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
    enum_fields: &std::collections::HashSet<String>,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "java";
    let call_overrides = call_config.overrides.get(lang);
    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let effective_result_var = &call_config.result_var;
    let effective_args = &call_config.args;
    let function_name = effective_function_name.as_str();
    let result_var = effective_result_var.as_str();
    let args: &[crate::config::ArgMapping] = effective_args.as_slice();

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve per-fixture options_type: prefer the java call override, fall back to
    // class-level, then to any other language's options_type for the same call (the
    // generated Java POJO class name matches the Rust type name across bindings, so
    // mirroring the C/csharp/go option lets us auto-emit `Type.fromJson(json)` without
    // requiring an explicit Java override per call).
    let effective_options_type: Option<String> = call_overrides
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()))
        .or_else(|| {
            // Borrow from any other backend's options_type. Prefer non-language-prefixed
            // names (csharp/c/go/php/python) over wasm or ruby which use prefixed types
            // like `WasmCreateBatchRequest` or `LiterLlm::CreateBatchRequest`.
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = call_config.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        return Some(t.clone());
                    }
                }
            }
            None
        });
    let effective_options_type = effective_options_type.as_deref();
    // When options_type is resolvable but no explicit options_via is given for Java,
    // default to "from_json" so the typed-request arg is emitted as
    // `Type.fromJson(json)` rather than the raw JSON string. The Java backend exposes
    // a static `fromJson(String)` factory on every record type (Stage A).
    let auto_from_json = effective_options_type.is_some()
        && call_overrides.and_then(|o| o.options_via.as_deref()).is_none()
        && e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.options_via.as_deref())
            .is_none();

    // Resolve client_factory: prefer call-level java override, fall back to file-level java override.
    let client_factory: Option<String> = call_overrides.and_then(|o| o.client_factory.clone()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.clone())
    });

    // Resolve options_via: "kwargs" (default), "from_json", "json", "dict".
    // Auto-default to "from_json" when an options_type is resolvable and no explicit
    // options_via is configured — this lets typed-request args emit `Type.fromJson(json)`
    // even when alef.toml only declares the type in another binding's override block.
    let options_via: String = call_overrides
        .and_then(|o| o.options_via.clone())
        .or_else(|| e2e_config.call.overrides.get(lang).and_then(|o| o.options_via.clone()))
        .unwrap_or_else(|| {
            if auto_from_json {
                "from_json".to_string()
            } else {
                "kwargs".to_string()
            }
        });

    // Resolve per-fixture result_is_simple and result_is_bytes from the call override.
    let effective_result_is_simple =
        call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple || result_is_simple;
    let effective_result_is_bytes = call_overrides.is_some_and(|o| o.result_is_bytes);

    // Check if this test needs ObjectMapper deserialization for json_object args.
    let needs_deser = effective_options_type.is_some()
        && args.iter().any(|arg| {
            if arg.arg_type != "json_object" {
                return false;
            }
            let val = super::resolve_field(&fixture.input, &arg.field);
            !val.is_null() && !val.is_array()
        });

    // Emit builder expressions for json_object args.
    let mut builder_expressions = String::new();
    if let (true, Some(opts_type)) = (needs_deser, effective_options_type) {
        for arg in args {
            if arg.arg_type == "json_object" {
                let val = super::resolve_field(&fixture.input, &arg.field);
                if !val.is_null() && !val.is_array() {
                    if options_via == "from_json" {
                        // Build the typed POJO via static fromJson(String) method.
                        let json_str = serde_json::to_string(val).unwrap_or_default();
                        let escaped = escape_java(&json_str);
                        let var_name = &arg.name;
                        builder_expressions.push_str(&format!(
                            "        var {var_name} = {opts_type}.fromJson(\"{escaped}\");\n",
                        ));
                    } else if let Some(obj) = val.as_object() {
                        // Generate builder expression: TypeName.builder().withFieldName(value)...build()
                        let empty_path_fields: Vec<String> = Vec::new();
                        let path_fields = call_overrides.map(|o| &o.path_fields).unwrap_or(&empty_path_fields);
                        let builder_expr = java_builder_expression(
                            obj,
                            opts_type,
                            enum_fields,
                            nested_types,
                            nested_types_optional,
                            path_fields,
                        );
                        let var_name = &arg.name;
                        builder_expressions.push_str(&format!("        var {} = {};\n", var_name, builder_expr));
                    }
                }
            }
        }
    }

    let (mut setup_lines, args_str) =
        build_args_and_setup(&fixture.input, args, class_name, effective_options_type, &fixture.id);

    // Per-language `extra_args` from call overrides — verbatim trailing
    // expressions appended after the configured args (e.g. `null` for an
    // optional trailing parameter the fixture cannot supply). Mirrors the
    // TypeScript and C# implementations.
    let extra_args_slice: &[String] = call_overrides.map_or(&[], |o| o.extra_args.as_slice());

    // Build visitor if present and add to setup
    let mut visitor_var = String::new();
    let mut has_visitor_fixture = false;
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_var = build_java_visitor(&mut setup_lines, visitor_spec, class_name);
        has_visitor_fixture = true;
    }

    // When visitor is present, attach it to the options parameter
    let mut final_args = if has_visitor_fixture {
        if args_str.is_empty() {
            format!("new ConversionOptions().withVisitor({})", visitor_var)
        } else if args_str.contains("new ConversionOptions")
            || args_str.contains("ConversionOptionsBuilder")
            || args_str.contains(".builder()")
        {
            // Options are being built (either new ConversionOptions(), builder pattern, or .builder().build())
            // append .withVisitor() call before .build() if present
            if args_str.contains(".build()") {
                let idx = args_str.rfind(".build()").unwrap();
                format!("{}.withVisitor({}){}", &args_str[..idx], visitor_var, &args_str[idx..])
            } else {
                format!("{}.withVisitor({})", args_str, visitor_var)
            }
        } else if args_str.ends_with(", null") {
            let base = &args_str[..args_str.len() - 6];
            format!("{}, new ConversionOptions().withVisitor({})", base, visitor_var)
        } else {
            format!("{}, new ConversionOptions().withVisitor({})", args_str, visitor_var)
        }
    } else {
        args_str
    };

    if !extra_args_slice.is_empty() {
        let extra_str = extra_args_slice.join(", ");
        final_args = if final_args.is_empty() {
            extra_str
        } else {
            format!("{final_args}, {extra_str}")
        };
    }

    // Render assertions_body
    let mut assertions_body = String::new();

    // Emit a `source` variable for run_query assertions that need the raw bytes.
    let needs_source_var = fixture
        .assertions
        .iter()
        .any(|a| a.assertion_type == "method_result" && a.method.as_deref() == Some("run_query"));
    if needs_source_var {
        if let Some(source_arg) = args.iter().find(|a| a.field == "source_code") {
            let field = source_arg.field.strip_prefix("input.").unwrap_or(&source_arg.field);
            if let Some(val) = fixture.input.get(field) {
                let java_val = json_to_java(val);
                assertions_body.push_str(&format!("        var source = {}.getBytes();\n", java_val));
            }
        }
    }

    // Merge per-call java enum_fields with the file-level java enum_fields so that
    // call-specific enum-typed result fields (e.g. `choices[0].finish_reason` for
    // chat) trigger Optional<Enum> coercion even when the global override block
    // does not list them. Per-call entries take precedence.
    // Combine global enum_fields (HashSet) with per-call overrides (HashMap).
    let mut effective_enum_fields: std::collections::HashSet<String> = enum_fields.clone();
    if let Some(co) = call_overrides {
        for k in co.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            class_name,
            field_resolver,
            effective_result_is_simple,
            effective_result_is_bytes,
            &effective_enum_fields,
        );
    }

    let throws_clause = " throws Exception";

    // When client_factory is set, instantiate a client and dispatch the call as
    // a method on the client; otherwise call the static helper on `class_name`.
    let (client_setup_lines, call_target) = if let Some(factory) = client_factory.as_deref() {
        let factory_name = factory.to_lower_camel_case();
        let fixture_id = &fixture.id;
        let mut setup: Vec<String> = Vec::new();
        if fixture.mock_response.is_some() || fixture.http.is_some() {
            setup.push(format!(
                "String mockUrl = System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\";"
            ));
            setup.push(format!(
                "var client = {class_name}.{factory_name}(\"test-key\", mockUrl, null, null, null);"
            ));
        } else if let Some(api_key_var) = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref()) {
            setup.push(format!("String apiKey = System.getenv(\"{api_key_var}\");"));
            setup.push(format!(
                "org.junit.jupiter.api.Assumptions.assumeTrue(apiKey != null && !apiKey.isEmpty(), \"{api_key_var} not set\");"
            ));
            setup.push(format!("var client = {class_name}.{factory_name}(apiKey);"));
        } else {
            setup.push(format!("var client = {class_name}.{factory_name}(\"test-key\");"));
        }
        (setup, "client".to_string())
    } else {
        (Vec::new(), class_name.to_string())
    };

    // Prepend client setup before any other setup_lines.
    let combined_setup: Vec<String> = client_setup_lines.into_iter().chain(setup_lines).collect();

    let call_expr = format!("{call_target}.{function_name}({final_args})");

    let rendered = crate::template_env::render(
        "java/test_method.jinja",
        minijinja::context! {
            method_name => method_name,
            description => description,
            builder_expressions => builder_expressions,
            setup_lines => combined_setup,
            throws_clause => throws_clause,
            expects_error => expects_error,
            call_expr => call_expr,
            result_var => result_var,
            assertions_body => assertions_body,
        },
    );
    out.push_str(&rendered);
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
fn build_args_and_setup(
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
            setup_lines.push(format!(
                "String {} = System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\";",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a createEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("var {} = {class_name}.{constructor_name}(null);", arg.name,));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let name = &arg.name;
                setup_lines.push(format!(
                    "var {name}Config = MAPPER.readValue(\"{}\", CrawlConfig.class);",
                    escape_java(&json_str),
                ));
                setup_lines.push(format!(
                    "var {} = {class_name}.{constructor_name}({name}Config);",
                    arg.name,
                    name = name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let resolved = super::resolve_field(input, &arg.field);
        let val = if resolved.is_null() { None } else { Some(resolved) };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: emit positional null/default so the call
                // has the right arity. For json_object optional args, build an empty default object
                // so we get the right type rather than a raw null.
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
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" | "file_path" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0d".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" {
                    // Array json_object args: emit inline Java list expression.
                    // Check for batch item arrays first (element_type = BatchBytesItem/BatchFileItem).
                    if v.is_array() {
                        if let Some(elem_type) = &arg.element_type {
                            if elem_type == "BatchBytesItem" || elem_type == "BatchFileItem" {
                                parts.push(emit_java_batch_item_array(v, elem_type));
                                continue;
                            }
                        }
                        // Otherwise use element_type to emit the correct numeric literal suffix (f vs d).
                        let elem_type = arg.element_type.as_deref();
                        parts.push(json_to_java_typed(v, elem_type));
                        continue;
                    }
                    // Object json_object args with options_type: use pre-deserialized variable.
                    if options_type.is_some() {
                        parts.push(arg.name.clone());
                        continue;
                    }
                    parts.push(json_to_java(v));
                    continue;
                }
                // bytes args must be passed as byte[], not String.
                if arg.arg_type == "bytes" {
                    let val = json_to_java(v);
                    parts.push(format!("{val}.getBytes()"));
                    continue;
                }
                // file_path args must be wrapped in java.nio.file.Path.of().
                if arg.arg_type == "file_path" {
                    let val = json_to_java(v);
                    parts.push(format!("java.nio.file.Path.of({val})"));
                    continue;
                }
                parts.push(json_to_java(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_bytes: bool,
    enum_fields: &std::collections::HashSet<String>,
) {
    // Byte-buffer returns: emit length-based assertions instead of struct-field
    // accessors. The result is `byte[]`, which has no `isEmpty()`/struct-field methods.
    // Field paths on byte-buffer results (e.g. `audio`, `content`) are pseudo-fields
    // referencing the buffer itself — treat them the same as no-field assertions.
    if result_is_bytes {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                out.push_str(&format!(
                    "        assertTrue({result_var}.length > 0, \"expected non-empty value\");\n"
                ));
                return;
            }
            "is_empty" => {
                out.push_str(&format!(
                    "        assertEquals(0, {result_var}.length, \"expected empty value\");\n"
                ));
                return;
            }
            "count_equals" | "length_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!("        assertEquals({n}, {result_var}.length);\n"));
                }
                return;
            }
            "count_min" | "length_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!(
                        "        assertTrue({result_var}.length >= {n}, \"expected length >= {n}\");\n"
                    ));
                }
                return;
            }
            _ => {
                out.push_str(&format!(
                    "        // skipped: assertion type '{}' not supported on byte[] result\n",
                    assertion.assertion_type
                ));
                return;
            }
        }
    }

    // Handle synthetic/virtual fields that are computed rather than direct record accessors.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            // ---- ExtractionResult chunk-level computed predicates ----
            "chunks_have_content" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.content() != null && !c.content().isBlank())"
                );
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_content",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_heading_context" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.metadata().headingContext().isPresent())"
                );
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_heading_context",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.embedding() != null && !c.embedding().isEmpty())"
                );
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "first_chunk_starts_with_heading" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().findFirst().map(c -> c.metadata().headingContext().isPresent()).orElse(false)"
                );
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "first_chunk_heading",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // When result_is_simple=true the result IS List<List<Float>> (the raw embeddings list).
            // When result_is_simple=false the result has an .embeddings() accessor.
            "embedding_dimensions" => {
                // Dimension = size of the first embedding vector in the list.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let expr = format!("({embed_list}.isEmpty() ? 0 : {embed_list}.get(0).size())");
                let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embedding_dimensions",
                        assertion_type => assertion.assertion_type.as_str(),
                        expr => expr,
                        java_val => java_val,
                        field_name => f,
                    },
                ));
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                // These are validation predicates that require iterating the embedding matrix.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{embed_list}.stream().allMatch(e -> e != null && !e.isEmpty())")
                    }
                    "embeddings_finite" => {
                        format!("{embed_list}.stream().flatMap(java.util.Collection::stream).allMatch(Float::isFinite)")
                    }
                    "embeddings_non_zero" => {
                        format!("{embed_list}.stream().allMatch(e -> e.stream().anyMatch(v -> v != 0.0f))")
                    }
                    "embeddings_normalized" => format!(
                        "{embed_list}.stream().allMatch(e -> {{ double n = e.stream().mapToDouble(v -> v * v).sum(); return Math.abs(n - 1.0) < 1e-3; }})"
                    ),
                    _ => unreachable!(),
                };
                let assertion_kind = format!("embeddings_{}", f.strip_prefix("embeddings_").unwrap_or(f));
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => assertion_kind,
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- Fields not present on the Java ExtractionResult ----
            "keywords" | "keywords_count" => {
                out.push_str(&crate::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "keywords",
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- metadata not_empty / is_empty: Metadata is a required record, not Optional ----
            // Metadata has no .isEmpty() method; check that at least one optional field is present.
            "metadata" => {
                match assertion.assertion_type.as_str() {
                    "not_empty" | "is_empty" => {
                        out.push_str(&crate::template_env::render(
                            "java/synthetic_assertion.jinja",
                            minijinja::context! {
                                assertion_kind => "metadata",
                                assertion_type => assertion.assertion_type.as_str(),
                                result_var => result_var,
                            },
                        ));
                        return;
                    }
                    _ => {} // fall through to normal handling
                }
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            out.push_str(&crate::template_env::render(
                "java/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // Determine if this field is an enum type (no `.contains()` on enums in Java).
    // Check both the raw fixture field path and the resolved (aliased) path so that
    // `fields_enum` entries can use either form (e.g., `"assets[].category"` or the
    // resolved `"assets[].asset_category"`).
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    // Determine if this field is an array (List<T>) — needed to choose .toString() for
    // contains assertions, since List.contains(Object) uses equals() which won't match
    // strings against complex record types like StructureItem.
    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                let accessor = field_resolver.accessor(f, "java", result_var);
                let resolved = field_resolver.resolve(f);
                // Unwrap Optional fields with a type-appropriate fallback.
                // Map.get() returns nullable, not Optional, so skip .orElse() for map access.
                // NOTE: is_optional() means the field is in optional_fields, but that doesn't
                // guarantee it returns Optional<T> in Java — nested fields like metadata.twitterCard
                // return @Nullable String, not Optional<String>. We detect this by checking
                // if the field path contains a dot (nested access).
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    // All nullable fields in the Java binding return @Nullable types, not Optional<T>.
                    // Wrap them in Optional.ofNullable() so e2e tests can use .orElse() fallbacks.
                    let optional_expr = format!("java.util.Optional.ofNullable({accessor})");
                    // Enum-typed optional fields need .map(v -> v.getValue()) to coerce to String
                    // before the orElse("") fallback can type-check (Optional<Enum>.orElse("") would
                    // be a type mismatch — Optional<String>.orElse("") is the only safe form).
                    if field_is_enum {
                        match assertion.assertion_type.as_str() {
                            "not_empty" | "is_empty" => optional_expr,
                            _ => format!("{optional_expr}.map(v -> v.getValue()).orElse(\"\")"),
                        }
                    } else {
                        match assertion.assertion_type.as_str() {
                            // For not_empty / is_empty on Optional fields, return the raw Optional
                            // so the assertion arms can call isPresent()/isEmpty().
                            "not_empty" | "is_empty" => optional_expr,
                            // For size/count assertions on Optional<List<T>> fields, use List.of() fallback.
                            "count_min" | "count_equals" => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            // For numeric comparisons on Optional<Long/Integer> fields, use 0L.
                            "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                                if field_resolver.is_array(resolved) {
                                    format!("{optional_expr}.orElse(java.util.List.of())")
                                } else {
                                    format!("{optional_expr}.orElse(0L)")
                                }
                            }
                            // For equals on Optional fields, determine fallback based on whether value is numeric.
                            // If the fixture value is a number, use 0L; otherwise use "".
                            "equals" => {
                                if let Some(expected) = &assertion.value {
                                    if expected.is_number() {
                                        format!("{optional_expr}.orElse(0L)")
                                    } else {
                                        format!("{optional_expr}.orElse(\"\")")
                                    }
                                } else {
                                    format!("{optional_expr}.orElse(\"\")")
                                }
                            }
                            _ if field_resolver.is_array(resolved) => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            _ => format!("{optional_expr}.orElse(\"\")"),
                        }
                    }
                } else {
                    accessor
                }
            }
            _ => result_var.to_string(),
        }
    };

    // For enum fields, string-based assertions need .getValue() to convert the enum to
    // its serde-serialized lowercase string value (e.g., AssetCategory.Image -> "image").
    // All alef-generated Java enums expose a getValue() method annotated with @JsonValue.
    // Optional enum fields are already coerced to String via `.map(v -> v.getValue()).orElse("")`
    // upstream in field_expr; in that case the value is already a String and we must not
    // call .getValue() again. Detect by looking for `.map(v -> v.getValue())` in the expr.
    let string_expr = if field_is_enum && !field_expr.contains(".map(v -> v.getValue())") {
        format!("{field_expr}.getValue()")
    } else {
        field_expr.clone()
    };

    // Pre-compute context for template
    let assertion_type = assertion.assertion_type.as_str();
    let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let is_numeric_val = assertion.value.as_ref().is_some_and(|v| v.is_number());

    let values_java: Vec<String> = assertion
        .values
        .as_ref()
        .map(|values| values.iter().map(json_to_java).collect())
        .unwrap_or_default();

    let contains_any_expr = if !values_java.is_empty() {
        values_java
            .iter()
            .map(|v| format!("{string_expr}.contains({v})"))
            .collect::<Vec<_>>()
            .join(" || ")
    } else {
        String::new()
    };

    let length_expr = if result_is_bytes {
        format!("{field_expr}.length")
    } else {
        format!("{field_expr}.length()")
    };

    let n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let call_expr = if let Some(method_name) = &assertion.method {
        build_java_method_call(result_var, method_name, assertion.args.as_ref(), class_name)
    } else {
        String::new()
    };

    let check = assertion.check.as_deref().unwrap_or("is_true");

    let java_check_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();

    let check_n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let is_bool_val = assertion.value.as_ref().is_some_and(|v| v.is_boolean());
    let bool_is_true = assertion.value.as_ref().is_some_and(|v| v.as_bool() == Some(true));

    let method_returns_collection = assertion
        .method
        .as_ref()
        .is_some_and(|m| matches!(m.as_str(), "find_nodes_by_type" | "findNodesByType"));

    let rendered = crate::template_env::render(
        "java/assertion.jinja",
        minijinja::context! {
            assertion_type,
            java_val,
            string_expr,
            field_expr,
            field_is_enum,
            field_is_array,
            is_string_val,
            is_numeric_val,
            values_java => values_java,
            contains_any_expr,
            length_expr,
            n,
            call_expr,
            check,
            java_check_val,
            check_n,
            is_bool_val,
            bool_is_true,
            method_returns_collection,
        },
    );
    out.push_str(&rendered);
}

/// Build a Java call expression for a `method_result` assertion on a tree-sitter Tree.
///
/// Maps method names to the appropriate Java static/instance method calls.
fn build_java_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    class_name: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode().childCount()"),
        "root_node_type" => format!("{result_var}.rootNode().kind()"),
        "named_children_count" => format!("{result_var}.rootNode().namedChildCount()"),
        "has_error_nodes" => format!("{class_name}.treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{class_name}.treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("{class_name}.treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.findNodesByType({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let escaped_query = escape_java(query_source);
            format!("{class_name}.runQuery({result_var}, \"{language}\", \"{escaped_query}\", source)")
        }
        _ => {
            format!("{result_var}.{}()", method_name.to_lower_camel_case())
        }
    }
}

/// Convert a `serde_json::Value` to a Java literal string.
fn json_to_java(value: &serde_json::Value) -> String {
    json_to_java_typed(value, None)
}

/// Convert a JSON value to a Java literal, optionally overriding number type for array elements.
/// `element_type` controls how numeric array elements are emitted: "f32" → `1.0f`, otherwise `1.0d`.
/// Emit Java batch item constructors for BatchBytesItem or BatchFileItem arrays.
fn emit_java_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let Some(obj) = item.as_object() {
                    match elem_type {
                        "BatchBytesItem" => {
                            let content = obj.get("content").and_then(|v| v.as_array());
                            let mime_type = obj.get("mime_type").and_then(|v| v.as_str()).unwrap_or("text/plain");
                            let content_code = if let Some(arr) = content {
                                let bytes: Vec<String> = arr
                                    .iter()
                                    .filter_map(|v| v.as_u64().map(|n| format!("(byte) {}", n)))
                                    .collect();
                                format!("new byte[] {{{}}}", bytes.join(", "))
                            } else {
                                "new byte[] {}".to_string()
                            };
                            Some(format!("new {}({}, \"{}\", null)", elem_type, content_code, mime_type))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            Some(format!(
                                "new {}(java.nio.file.Paths.get(\"{}\"), null)",
                                elem_type, path
                            ))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("java.util.Arrays.asList({})", item_strs.join(", "))
    } else {
        "java.util.List.of()".to_string()
    }
}

fn json_to_java_typed(value: &serde_json::Value, element_type: Option<&str>) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_java(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                match element_type {
                    Some("f32" | "float" | "Float") => format!("{}f", n),
                    _ => format!("{}d", n),
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, element_type)).collect();
            format!("java.util.List.of({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_java(&json_str))
        }
    }
}

/// Generate a Java builder expression for a JSON object.
/// E.g., `obj = {"language": "abl", "chunk_max_size": 50}`
/// becomes: `TypeName.builder().withLanguage("abl").withChunkMaxSize(50L).build()`
///
/// For enums: emit `EnumType.VariantName` (detected via camelCase lookup in enum_fields)
/// For strings and bools: use the value directly
/// For plain numbers: emit the literal with type suffix (long uses L, double uses d)
/// For nested objects: recurse with Options suffix
/// When `nested_types_optional` is false, nested builders are passed directly without
/// Optional.of() wrapping, allowing non-optional nested config types.
fn java_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_fields: &std::collections::HashSet<String>,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    path_fields: &[String],
) -> String {
    let mut expr = format!("{}.builder()", type_name);
    for (key, val) in obj {
        // Convert snake_case key to camelCase for method name
        let camel_key = key.to_lower_camel_case();
        let method_name = format!("with{}", camel_key.to_upper_camel_case());

        let java_val = match val {
            serde_json::Value::String(s) => {
                // Check if this field is an enum type by checking enum_fields.
                // Infer enum type name from camelCase field name by converting to UpperCamelCase.
                if enum_fields.contains(&camel_key) {
                    // Enum field: infer type name from field name (e.g., "codeBlockStyle" -> "CodeBlockStyle")
                    let enum_type_name = camel_key.to_upper_camel_case();
                    let variant_name = s.to_upper_camel_case();
                    format!("{}.{}", enum_type_name, variant_name)
                } else if camel_key == "preset" && type_name == "PreprocessingOptions" {
                    // Special case: preset field in PreprocessingOptions maps to PreprocessingPreset
                    let variant_name = s.to_upper_camel_case();
                    format!("PreprocessingPreset.{}", variant_name)
                } else if path_fields.contains(key) {
                    // Path field: wrap in Optional.of(java.nio.file.Path.of(...))
                    format!("Optional.of(java.nio.file.Path.of(\"{}\"))", escape_java(s))
                } else {
                    // String field: emit as a quoted literal
                    format!("\"{}\"", escape_java(s))
                }
            }
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Number(n) => {
                // Number field: emit literal with type suffix.
                // Java records/classes use either `long` (primitive, not nullable) or
                // `Optional<Long>` (nullable). The codegen wraps in `Optional.of(...)`
                // by default since most options builder fields are Optional, but several
                // record types (e.g. SecurityLimits) use primitive `long` throughout.
                // Skip the wrap for: (a) known-primitive top-level fields and (b) any
                // method on a record type whose builder methods take primitives only.
                let camel_key = key.to_lower_camel_case();
                let is_plain_field = matches!(camel_key.as_str(), "listIndentWidth" | "wrapWidth");
                // Builders for typed-record nested config classes use primitives
                // throughout — they're not the optional-options pattern.
                let is_primitive_builder = matches!(type_name, "SecurityLimits" | "SecurityLimitsBuilder");

                if is_plain_field || is_primitive_builder {
                    // Plain numeric field: no Optional wrapper
                    if n.is_f64() {
                        format!("{}d", n)
                    } else {
                        format!("{}L", n)
                    }
                } else {
                    // Optional numeric field: wrap in Optional.of()
                    if n.is_f64() {
                        format!("Optional.of({}d)", n)
                    } else {
                        format!("Optional.of({}L)", n)
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, None)).collect();
                format!("java.util.List.of({})", items.join(", "))
            }
            serde_json::Value::Object(nested) => {
                // Recurse with the type from nested_types mapping, or default to snake_case → PascalCase + "Options".
                let nested_type = nested_types
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("{}Options", key.to_upper_camel_case()));
                let inner = java_builder_expression(
                    nested,
                    &nested_type,
                    enum_fields,
                    nested_types,
                    nested_types_optional,
                    &[],
                );
                // Top-level config builders (e.g. ExtractionConfigBuilder) declare nested
                // record fields as `Optional<T>` (since they are nullable). Primitive-fields
                // builders (SecurityLimitsBuilder etc.) take the bare type directly.
                let is_primitive_builder = matches!(type_name, "SecurityLimits" | "SecurityLimitsBuilder");
                if is_primitive_builder || !nested_types_optional {
                    inner
                } else {
                    format!("Optional.of({inner})")
                }
            }
        };
        expr.push_str(&format!(".{}({})", method_name, java_val));
    }
    expr.push_str(".build()");
    expr
}

/// Build default nested type mappings for Java extraction config types.
///
/// Maps known Kreuzberg/Kreuzcrawl config field names (in snake_case) to their
/// Java record type names (in PascalCase). These defaults allow e2e codegen to
/// automatically deserialize nested config objects without requiring explicit
/// configuration in alef.toml. User-provided overrides take precedence.
fn default_java_nested_types() -> std::collections::HashMap<String, String> {
    [
        ("chunking", "ChunkingConfig"),
        ("ocr", "OcrConfig"),
        ("images", "ImageExtractionConfig"),
        ("html_output", "HtmlOutputConfig"),
        ("language_detection", "LanguageDetectionConfig"),
        ("postprocessor", "PostProcessorConfig"),
        ("acceleration", "AccelerationConfig"),
        ("email", "EmailConfig"),
        ("pages", "PageConfig"),
        ("pdf_options", "PdfConfig"),
        ("layout", "LayoutDetectionConfig"),
        ("tree_sitter", "TreeSitterConfig"),
        ("structured_extraction", "StructuredExtractionConfig"),
        ("content_filter", "ContentFilterConfig"),
        ("token_reduction", "TokenReductionOptions"),
        ("security_limits", "SecurityLimits"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

// ---------------------------------------------------------------------------
// Import collection helpers
// ---------------------------------------------------------------------------

/// Recursively collect enum types and nested option types used in a builder expression.
/// Enums are keyed in the enum_fields map by camelCase names (e.g., "codeBlockStyle" → "CodeBlockStyle").
#[allow(dead_code)]
fn collect_enum_and_nested_types(
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        // enum_fields is keyed by camelCase, not snake_case.
        let camel_key = key.to_lower_camel_case();
        if let Some(enum_type) = enum_fields.get(&camel_key) {
            // Add the enum type from the mapping (e.g., "CodeBlockStyle").
            types_out.insert(enum_type.clone());
        } else if camel_key == "preset" {
            // Special case: preset field uses PreprocessingPreset enum.
            types_out.insert("PreprocessingPreset".to_string());
        }
        // Recurse into nested objects to find their nested enum types.
        if let Some(nested) = val.as_object() {
            collect_enum_and_nested_types(nested, enum_fields, types_out);
        }
    }
}

fn collect_nested_type_names(
    obj: &serde_json::Map<String, serde_json::Value>,
    nested_types: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        if let Some(type_name) = nested_types.get(key.as_str()) {
            types_out.insert(type_name.clone());
        }
        if let Some(nested) = val.as_object() {
            collect_nested_type_names(nested, nested_types, types_out);
        }
    }
}

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a Java visitor class and add setup lines. Returns the visitor variable name.
fn build_java_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &crate::fixture::VisitorSpec,
    class_name: &str,
) -> String {
    setup_lines.push("class _TestVisitor implements Visitor {".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_java_visitor_method(setup_lines, method_name, action, class_name);
    }
    setup_lines.push("}".to_string());
    setup_lines.push("var visitor = new _TestVisitor();".to_string());
    "visitor".to_string()
}

/// Emit a Java visitor method for a callback action.
fn emit_java_visitor_method(
    setup_lines: &mut Vec<String>,
    method_name: &str,
    action: &CallbackAction,
    _class_name: &str,
) {
    let camel_method = method_to_camel(method_name);
    let params = match method_name {
        "visit_link" => "NodeContext ctx, String href, String text, String title",
        "visit_image" => "NodeContext ctx, String src, String alt, String title",
        "visit_heading" => "NodeContext ctx, int level, String text, String id",
        "visit_code_block" => "NodeContext ctx, String lang, String code",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "NodeContext ctx, String text",
        "visit_text" => "NodeContext ctx, String text",
        "visit_list_item" => "NodeContext ctx, boolean ordered, String marker, String text",
        "visit_blockquote" => "NodeContext ctx, String content, long depth",
        "visit_table_row" => "NodeContext ctx, java.util.List<String> cells, boolean isHeader",
        "visit_custom_element" => "NodeContext ctx, String tagName, String html",
        "visit_form" => "NodeContext ctx, String actionUrl, String method",
        "visit_input" => "NodeContext ctx, String inputType, String name, String value",
        "visit_audio" | "visit_video" | "visit_iframe" => "NodeContext ctx, String src",
        "visit_details" => "NodeContext ctx, boolean isOpen",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "NodeContext ctx, String output"
        }
        "visit_list_start" => "NodeContext ctx, boolean ordered",
        "visit_list_end" => "NodeContext ctx, boolean ordered, String output",
        _ => "NodeContext ctx",
    };

    // Determine action type and values for template
    let (action_type, action_value, format_args) = match action {
        CallbackAction::Skip => ("skip", String::new(), Vec::new()),
        CallbackAction::Continue => ("continue", String::new(), Vec::new()),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), Vec::new()),
        CallbackAction::Custom { output } => ("custom_literal", escape_java(output), Vec::new()),
        CallbackAction::CustomTemplate { template } => {
            // Extract {placeholder} names from the template (in order of appearance).
            let mut format_str = String::with_capacity(template.len());
            let mut format_args: Vec<String> = Vec::new();
            let mut chars = template.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '{' {
                    // Collect identifier chars until '}'.
                    let mut name = String::new();
                    let mut closed = false;
                    for inner in chars.by_ref() {
                        if inner == '}' {
                            closed = true;
                            break;
                        }
                        name.push(inner);
                    }
                    if closed && !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        let camel_name = name.as_str().to_lower_camel_case();
                        format_args.push(camel_name);
                        format_str.push_str("%s");
                    } else {
                        // Not a simple placeholder — emit literally.
                        format_str.push('{');
                        format_str.push_str(&name);
                        if closed {
                            format_str.push('}');
                        }
                    }
                } else {
                    format_str.push(ch);
                }
            }
            let escaped = escape_java(&format_str);
            if format_args.is_empty() {
                ("custom_literal", escaped, Vec::new())
            } else {
                ("custom_formatted", escaped, format_args)
            }
        }
    };

    let params = params.to_string();

    let rendered = crate::template_env::render(
        "java/visitor_method.jinja",
        minijinja::context! {
            camel_method,
            params,
            action_type,
            action_value,
            format_args => format_args,
        },
    );
    setup_lines.push(rendered);
}

/// Convert snake_case method names to Java camelCase.
fn method_to_camel(snake: &str) -> String {
    snake.to_lower_camel_case()
}
