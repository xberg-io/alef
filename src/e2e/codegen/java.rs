//! Java e2e test generator using JUnit 5.
//!
//! Generates `e2e/java/pom.xml` and language-package test classes
//! files from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_java, sanitize_filename};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture};
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;
use super::java_mvnw::{MAVEN_WRAPPER_PROPERTIES, MVNW_UNIX, MVNW_WINDOWS};

/// Check if a type name is a numeric type hint (f32, float, etc.) vs. a complex type name.
fn is_numeric_type_hint(ty: &str) -> bool {
    matches!(ty, "f32" | "f64" | "float" | "double" | "Float" | "Double")
}

/// Check if a type name is a Java built-in type that doesn't need an import.
fn is_java_builtin_type(ty: &str) -> bool {
    matches!(
        ty,
        "String" | "Boolean" | "Integer" | "Long" | "Double" | "Float" | "Byte" | "Short" | "Character" | "Void"
    )
}

fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

/// Java e2e code generator.
pub struct JavaCodegen;

impl E2eCodegen for JavaCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
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

        // Maven wrapper: ./mvnw + mvnw.cmd + .mvn/wrapper/maven-wrapper.properties.
        // The wrapper scripts bootstrap-download maven-wrapper.jar from the URL in
        // maven-wrapper.properties on first invocation, so alef does not need to
        // emit the binary jar. The shebang on mvnw triggers 0755 chmod in the
        // file writer.
        files.push(GeneratedFile {
            path: output_base.join("mvnw"),
            content: MVNW_UNIX.to_string(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base.join("mvnw.cmd"),
            content: MVNW_WINDOWS.to_string(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base
                .join(".mvn")
                .join("wrapper")
                .join("maven-wrapper.properties"),
            content: MAVEN_WRAPPER_PROPERTIES.to_string(),
            generated_header: false,
        });

        // Check if there are HTTP fixtures that need server-pattern harness
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();
        // Detect mock-server need from fixture `mock_response` or `http.expected_response`
        // shapes. Mirrors kotlin_android codegen.
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

        // When any fixture needs a mock server, emit MockServerListener.java
        // plus its META-INF SPI entry so JUnit Platform discovers and starts
        // the `mock-server` binary once per launcher session. Without these
        // the tests reference `mockServerUrl` but no server runs, and the
        // existing service file (if left over from a prior alef version) points
        // at a class that does not exist on the classpath.
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

        // Emit fixture JSON files to src/test/resources/fixtures/ (avoids 65KB string literal limit)
        let fixtures_resource_base = output_base.join("src").join("test").join("resources").join("fixtures");
        for group in groups {
            for fixture in &group.fixtures {
                if fixture.http.is_none() {
                    continue;
                }
                let http_data = fixture.http.as_ref().unwrap();
                let fixture_json = serde_json::json!({
                    "http": {
                        "handler": {
                            "route": &http_data.handler.route,
                            "method": &http_data.handler.method,
                            "body_schema": http_data.handler.body_schema.clone(),
                        },
                        "request": {
                            "path": &http_data.request.path,
                        },
                        "expected_response": {
                            "status_code": http_data.expected_response.status_code,
                            "body": &http_data.expected_response.body,
                            "headers": &http_data.expected_response.headers,
                        }
                    }
                });
                let fixture_json_str = serde_json::to_string(&fixture_json).unwrap_or_default();
                files.push(GeneratedFile {
                    path: fixtures_resource_base.join(format!("{}.json", fixture.id)),
                    content: fixture_json_str,
                    generated_header: false,
                });
            }
        }

        // Emit FixtureLoader.java helper for loading fixtures from classpath
        if uses_harness {
            files.push(GeneratedFile {
                path: test_base.join("FixtureLoader.java"),
                content: render_fixture_loader(&java_group_id),
                generated_header: true,
            });
        }

        // Emit HarnessMain.java if server-pattern harness is needed
        if uses_harness {
            files.push(GeneratedFile {
                path: test_base.join("HarnessMain.java"),
                content: render_harness_main(e2e_config, groups, &java_group_id, &binding_pkg),
                generated_header: true,
            });
        }

        // Collect all distinct sealed-union type names declared in `assert_enum_fields`
        // across all call configs for this language.  For each such type we emit a
        // `{TypeName}Display.java` helper that pattern-matches on variants from the IR;
        // projects that declare no `assert_enum_fields` get no extra helper files.
        let sealed_display_types: std::collections::BTreeSet<String> = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .filter_map(|c| c.overrides.get(lang))
            .flat_map(|o| o.assert_enum_fields.values().cloned())
            .collect();

        for type_name in &sealed_display_types {
            if let Some(enum_def) = enums.iter().find(|e| &e.name == type_name) {
                files.push(GeneratedFile {
                    path: test_base.join(format!("{type_name}Display.java")),
                    content: render_sealed_display(type_name, enum_def, type_defs, &java_group_id),
                    generated_header: true,
                });
            }
        }

        // Resolve options_type: prefer Java override, fall back to other languages' options_type.
        // This ensures that when a call declares options_type in C#/Go/Python/PHP but not Java,
        // Java e2e tests still properly deserialize json_object args via JsonUtil.fromJson().
        let options_type = overrides.and_then(|o| o.options_type.clone()).or_else(|| {
            // Inherit from non-Java language overrides (C# first, then C, Go, PHP, Python).
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = e2e_config.call.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        return Some(t.clone());
                    }
                }
            }
            None
        });

        // Resolve enum_fields and nested_types from Java override config.
        static EMPTY_ENUM_FIELDS: std::sync::LazyLock<std::collections::HashMap<String, String>> =
            std::sync::LazyLock::new(std::collections::HashMap::new);
        let _enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&EMPTY_ENUM_FIELDS);

        // Build effective nested_types from configured overrides (empty by default).
        let mut effective_nested_types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(overrides_map) = overrides.map(|o| &o.nested_types) {
            effective_nested_types.extend(overrides_map.clone());
        }

        // Resolve nested_types_optional from override (defaults to true for backward compatibility).
        let nested_types_optional = overrides.map(|o| o.nested_types_optional).unwrap_or(true);

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
                result_is_simple,
                e2e_config,
                &effective_nested_types,
                nested_types_optional,
                &config.adapters,
                config,
                type_defs,
                uses_harness,
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
    dep_mode: crate::e2e::config::DependencyMode,
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
        crate::e2e::config::DependencyMode::Registry => {
            format!(
                r#"        <dependency>
            <groupId>{dep_group_id}</groupId>
            <artifactId>{dep_artifact_id}</artifactId>
            <version>{pkg_version}</version>
        </dependency>"#
            )
        }
        crate::e2e::config::DependencyMode::Local => {
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
    // Registry-mode test_apps consume the published Maven Central JAR, which
    // bundles natives under `/natives/{rid}/`; NativeLib extracts and loads
    // them at startup without needing java.library.path. Local-mode e2e tests
    // depend on a locally-built JAR that does NOT bundle natives, and must
    // resolve the shared library from a separate cargo build output.
    let include_native_lib_path = matches!(dep_mode, crate::e2e::config::DependencyMode::Local);
    crate::e2e::template_env::render(
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
            include_native_lib_path => include_native_lib_path,
        },
    )
}

/// Render HarnessMain.java for server-pattern e2e tests.
///
/// This harness loads fixtures from classpath resources, registers handlers via
/// the app binding, and serves on a port read from SUT_URL env var or the
/// configured default. Tests hit the real SUT at /fixtures/<fixture_id>{path}.
fn render_harness_main(
    e2e_config: &E2eConfig,
    groups: &[FixtureGroup],
    java_group_id: &str,
    binding_pkg: &str,
) -> String {
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;
    let app_class_owned = e2e_config.harness.app_class_for_lang("java");
    let app_class = app_class_owned.as_deref().unwrap_or("App");
    let run_method_owned = e2e_config.harness.run_method_for_lang("java");
    let run_method = run_method_owned.as_deref().unwrap_or("run");
    // Java methods are camelCase by convention. `register_method_idiomatic`
    // honors `[crates.e2e.harness.overrides.java]` first, then converts the
    // canonical name to camelCase (e.g. `register_route` → `registerRoute`).
    let register_method = e2e_config
        .harness
        .register_method_idiomatic("java")
        .unwrap_or_else(|| "registerAppRoute".to_string());
    let body_field = &e2e_config.harness.response_body_field;

    // Collect all HTTP fixtures for this harness to register.
    let mut fixture_ids: Vec<String> = Vec::new();
    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_some() {
                fixture_ids.push(fixture.id.clone());
            }
        }
    }

    let ctx = minijinja::context! {
        java_group_id => java_group_id,
        binding_pkg => binding_pkg,
        app_class => app_class,
        run_method => run_method,
        register_method => register_method.as_str(),
        response_body_field => body_field.as_str(),
        host => host,
        port => port,
        fixture_ids => fixture_ids,
    };

    crate::e2e::template_env::render("java/harness_main.jinja", ctx)
}

/// Render FixtureLoader.java helper that loads fixture JSON files from classpath.
///
/// This avoids inlining all fixtures as Java string literals, which would exceed
/// Java's 65535-byte limit for large fixture sets. Fixtures are stored as individual
/// JSON files in src/test/resources/fixtures/ and loaded at test runtime.
fn render_fixture_loader(java_group_id: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("package {java_group_id}.e2e;\n\n"));
    out.push_str("import com.fasterxml.jackson.databind.JsonNode;\n");
    out.push_str("import com.fasterxml.jackson.databind.ObjectMapper;\n");
    out.push_str("import java.io.IOException;\n");
    out.push_str("import java.io.InputStream;\n");
    out.push_str("import java.util.HashMap;\n");
    out.push_str("import java.util.Map;\n");
    out.push('\n');
    out.push_str("/**\n");
    out.push_str(" * Helper class for loading fixture JSON files from classpath.\n");
    out.push_str(" *\n");
    out.push_str(" * Fixtures are stored as individual JSON files in src/test/resources/fixtures/\n");
    out.push_str(" * to avoid exceeding Java's 65KB string literal limit.\n");
    out.push_str(" */\n");
    out.push_str("public class FixtureLoader {\n");
    out.push_str("    private static final ObjectMapper MAPPER = new ObjectMapper();\n");
    out.push('\n');
    out.push_str("    /**\n");
    out.push_str("     * Load a single fixture by ID from classpath resources.\n");
    out.push_str("     *\n");
    out.push_str("     * @param fixtureId the fixture identifier (e.g., \"smoke_basic\")\n");
    out.push_str("     * @return the parsed fixture as a JsonNode, or null if not found\n");
    out.push_str("     */\n");
    out.push_str("    public static JsonNode loadFixture(String fixtureId) {\n");
    out.push_str("        String resourcePath = \"/fixtures/\" + fixtureId + \".json\";\n");
    out.push_str("        try (InputStream is = FixtureLoader.class.getResourceAsStream(resourcePath)) {\n");
    out.push_str("            if (is == null) {\n");
    out.push_str("                System.err.println(\"Fixture not found: \" + fixtureId);\n");
    out.push_str("                return null;\n");
    out.push_str("            }\n");
    out.push_str("            return MAPPER.readTree(is);\n");
    out.push_str("        } catch (IOException e) {\n");
    out.push_str(
        "            System.err.println(\"Failed to load fixture \" + fixtureId + \": \" + e.getMessage());\n",
    );
    out.push_str("            e.printStackTrace();\n");
    out.push_str("            return null;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    /**\n");
    out.push_str("     * Load all fixtures from the classpath resources directory.\n");
    out.push_str("     *\n");
    out.push_str("     * @return a map of fixture IDs to parsed fixture JsonNodes\n");
    out.push_str("     */\n");
    out.push_str("    public static Map<String, JsonNode> loadAllFixtures() {\n");
    out.push_str("        Map<String, JsonNode> fixtures = new HashMap<>();\n");
    out.push_str("        // Note: Loading all fixtures requires iterating the classpath.\n");
    out.push_str("        // For typical e2e test suites, only the fixtures needed by the\n");
    out.push_str("        // specific test class should be loaded via loadFixture(id).\n");
    out.push_str("        return fixtures;\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
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
    out.push_str("import java.util.regex.Matcher;\n");
    out.push_str("import java.util.regex.Pattern;\n");
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
    out.push_str("        // Read until we see MOCK_SERVER_URL= and optionally MOCK_SERVERS=.\n");
    out.push_str("        // Cap the loop so a misbehaving mock-server cannot block indefinitely.\n");
    out.push_str("        BufferedReader stdout = new BufferedReader(new InputStreamReader(mockServer.getInputStream(), StandardCharsets.UTF_8));\n");
    out.push_str("        String url = null;\n");
    out.push_str("        try {\n");
    out.push_str("            for (int i = 0; i < 16; i++) {\n");
    out.push_str("                String line = stdout.readLine();\n");
    out.push_str("                if (line == null) break;\n");
    out.push_str("                if (line.startsWith(\"MOCK_SERVER_URL=\")) {\n");
    out.push_str("                    url = line.substring(\"MOCK_SERVER_URL=\".length()).trim();\n");
    out.push_str("                } else if (line.startsWith(\"MOCK_SERVERS=\")) {\n");
    out.push_str("                    String jsonVal = line.substring(\"MOCK_SERVERS=\".length()).trim();\n");
    out.push_str("                    System.setProperty(\"mockServers\", jsonVal);\n");
    out.push_str("                    // Parse JSON map of fixture_id -> url and expose as system properties.\n");
    out.push_str("                    Pattern p = Pattern.compile(\"\\\"([^\\\"]+)\\\":\\\"([^\\\"]+)\\\"\");\n");
    out.push_str("                    Matcher matcher = p.matcher(jsonVal);\n");
    out.push_str("                    while (matcher.find()) {\n");
    out.push_str("                        String fid = matcher.group(1);\n");
    out.push_str("                        String furl = matcher.group(2);\n");
    out.push_str("                        System.setProperty(\"mockServer.\" + fid, furl);\n");
    out.push_str("                    }\n");
    out.push_str("                    break;\n");
    out.push_str("                } else if (url != null) {\n");
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

/// Generate a `{TypeName}Display.java` helper that pattern-matches on every
/// variant of a sealed interface and returns a display string for e2e assertions.
///
/// Variant dispatch logic:
/// - Tuple variants whose inner type (looked up in `type_defs`) has a field named
///   `format` emit `v.value().format()` so image-format strings (PNG, JPEG, …)
///   are returned rather than the literal variant name.
/// - All other variants emit the lowercased serde name (or lowercased variant name
///   when no serde rename is declared).
///
/// A `default -> "unknown"` catch-all is always appended so the generated code
/// remains forward-compatible when new variants are added to the Rust enum.
fn render_sealed_display(
    type_name: &str,
    enum_def: &crate::core::ir::EnumDef,
    type_defs: &[crate::core::ir::TypeDef],
    java_group_id: &str,
) -> String {
    let helper_class = format!("{type_name}Display");
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("package {java_group_id}.e2e;\n\n"));
    out.push_str(&format!("import {java_group_id}.{type_name};\n"));
    out.push('\n');
    out.push_str(&format!(
        "/**\n * Helper class for extracting display strings from {type_name} sealed interface.\n */\n"
    ));
    out.push_str(&format!("class {helper_class} {{\n"));
    out.push_str(&format!("    static String toDisplayString({type_name} value) {{\n"));
    out.push_str("        if (value == null) return \"\";\n");
    out.push_str("        return switch (value) {\n");

    for variant in &enum_def.variants {
        let variant_name = &variant.name;
        // Determine the display string for this variant's arm.
        // Tuple variants with one field whose resolved struct type has a `format`
        // field return the inner `.value().format()` — this gives the actual format
        // string (e.g. "PNG") rather than the generic variant label (e.g. "image").
        let has_format_field = variant.is_tuple && variant.fields.len() == 1 && {
            let field_type_name = match &variant.fields[0].ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            };
            field_type_name.is_some_and(|tn| {
                type_defs
                    .iter()
                    .find(|td| td.name == tn)
                    .is_some_and(|td| td.fields.iter().any(|f| f.name == "format"))
            })
        };

        let display = if has_format_field {
            "i.value().format()".to_string()
        } else {
            // Use the serde rename when present; otherwise lowercase the variant name.
            let serde_name = variant
                .serde_rename
                .as_deref()
                .unwrap_or(variant_name.as_str())
                .to_lowercase();
            format!("\"{serde_name}\"")
        };

        let binding = if has_format_field {
            format!("{type_name}.{variant_name} i")
        } else {
            format!("{type_name}.{variant_name} _")
        };

        out.push_str(&format!("            case {binding} -> {display};\n"));
    }

    out.push_str("            default -> \"unknown\";\n");
    out.push_str("        };\n");
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
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    uses_harness: bool,
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
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_om, f, call_cfg, type_defs);
        recipe.args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = super::resolve_field(&f.input, &a.field);
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
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
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
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang_for_om, f, call_cfg, type_defs);
        if f.visitor.is_some() {
            if let Some(binding) = java_visitor_binding(config, recipe.options_type) {
                all_options_types.insert(binding.options_type);
            }
        }
        for arg in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
            let value = super::resolve_field(&f.input, &arg.field);
            if value.is_null() || value.is_object() && value.as_object().is_some_and(|o| o.is_empty()) {
                continue;
            }
            if let Some(handle_type) = resolve_handle_config_type(arg, recipe.options_type, type_defs) {
                all_options_types.insert(handle_type);
            }
        }
        // Detect complex json_object array element types used in this fixture.
        for arg in &call_cfg.args {
            if let Some(elem_type) = &arg.element_type {
                if arg.arg_type == "json_object" && !is_numeric_type_hint(elem_type) && !is_java_builtin_type(elem_type)
                {
                    // Complex types in json_object arrays need JsonUtil.
                    // Skip Java built-in types (String, Boolean, Integer, etc.).
                    all_options_types.insert(elem_type.clone());
                }
            }
        }
    }

    // Collect nested config types actually referenced in fixture builder expressions.
    // Note: enum types don't need explicit imports since they're in the same package.
    let mut nested_types_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
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

    // Import visitor types when any fixture uses visitor callbacks.
    let has_visitor_fixtures = fixtures.iter().any(|f| f.visitor.is_some());
    if has_visitor_fixtures && !binding_pkg_for_imports.is_empty() {
        imports.push(format!("import {binding_pkg_for_imports}.Visitor;"));
        imports.push(format!("import {binding_pkg_for_imports}.NodeContext;"));
        imports.push(format!("import {binding_pkg_for_imports}.VisitResult;"));
    }

    // Import Optional when using builder expressions with optional fields.
    // Also import JsonUtil for `JsonUtil.fromJson(json, Type.class)` calls emitted when
    // options_via resolves to "from_json" (the default whenever an options_type is present).
    if !all_options_types.is_empty() {
        imports.push("import java.util.Optional;".to_string());
        if !binding_pkg_for_imports.is_empty() {
            imports.push(format!("import {binding_pkg_for_imports}.JsonUtil;"));
        }
    }

    // Import streaming DTOs when any fixture is streaming (uses chat_stream
    // or references streaming-virtual fields like `chunks`/`stream_content`).
    // The collect_snippet emits `new ArrayList<ItemType>()` so the item type
    // class must be importable for type inference and method resolution.
    //
    // Use `resolve_is_streaming` so per-call `streaming = false` opt-outs are
    // honoured: consumers like parser-language-pack ship a real `chunks`
    // result field on their non-streaming process result, and would otherwise
    // get a spurious import plus virtual-aggregator accessor expansion on
    // `chunks`-shaped assertions.
    let has_streaming_fixture = fixtures.iter().any(|f| {
        let call_cfg =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(f, call_cfg.streaming_enabled())
    });
    if has_streaming_fixture && !binding_pkg_for_imports.is_empty() {
        // Derive streaming DTO imports from declared adapters so each project pulls
        // in only the request and item types it actually exposes.
        let mut streaming_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for adapter in adapters {
            if !matches!(adapter.pattern, crate::core::config::extras::AdapterPattern::Streaming) {
                continue;
            }
            if let Some(item) = adapter.item_type.as_deref() {
                let simple = item.rsplit("::").next().unwrap_or(item);
                if !simple.is_empty() {
                    streaming_imports.insert(simple.to_string());
                }
            }
            if let Some(req) = adapter.request_type.as_deref() {
                let simple = req.rsplit("::").next().unwrap_or(req);
                if !simple.is_empty() {
                    streaming_imports.insert(simple.to_string());
                }
            }
        }
        for ty in streaming_imports {
            imports.push(format!("import {binding_pkg_for_imports}.{ty};"));
        }
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
            result_is_simple,
            e2e_config,
            nested_types,
            nested_types_optional,
            adapters,
            config,
            type_defs,
        );
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    // Render template
    crate::e2e::template_env::render(
        "java/test_file.jinja",
        minijinja::context! {
            header => header,
            java_group_id => java_group_id,
            test_class_name => test_class_name,
            category => category,
            imports => imports,
            needs_object_mapper => needs_object_mapper,
            fixtures_body => fixtures_body,
            uses_harness => uses_harness,
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
        let rendered = crate::e2e::template_env::render(
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
        let rendered = crate::e2e::template_env::render("java/http_test_close.jinja", minijinja::context! {});
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
                    // Percent-encode so values with spaces/reserved characters yield a valid
                    // URI literal (java.net.URI.create rejects raw spaces).
                    format!(
                        "{}={}",
                        super::percent_encode_query(k),
                        super::percent_encode_query(&val_str)
                    )
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

        let rendered = crate::e2e::template_env::render(
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
        let rendered = crate::e2e::template_env::render(
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

        let rendered = crate::e2e::template_env::render(
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

        let rendered = crate::e2e::template_env::render(
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

            let rendered = crate::e2e::template_env::render(
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
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
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

        let rendered = crate::e2e::template_env::render(
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
        out.push_str(&crate::e2e::template_env::render(
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
    _args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    // Use resolve_call_for_fixture to support auto-routing via select_when.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Per-call field resolver: overrides the category-level resolver when this call
    // declares its own result_fields / fields / fields_optional / fields_array.
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    );
    let field_resolver = &call_field_resolver;
    let effective_enum_fields = e2e_config.effective_fields_enum(call_config);
    let enum_fields = effective_enum_fields;
    let lang = "java";
    let call_overrides = call_config.overrides.get(lang);
    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let effective_result_var = &call_config.result_var;
    let function_name = effective_function_name.as_str();
    let result_var = effective_result_var.as_str();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args: &[crate::e2e::config::ArgMapping] = recipe.args;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve per-fixture options_type: prefer the java call override, fall back to
    // class-level, then to any other language's options_type for the same call (the
    // generated Java POJO class name matches the Rust type name across bindings, so
    // mirroring the C/csharp/go option lets us auto-emit `Type.fromJson(json)` without
    // requiring an explicit Java override per call).
    let effective_options_type: Option<String> = recipe
        .options_type
        .map(str::to_string)
        .or_else(|| options_type.map(str::to_string))
        .or_else(|| {
            recipe
                .compatible_options_type(&["csharp", "c", "go", "php", "python"])
                .map(str::to_string)
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
    // Resolve result_is_option: when the Rust function returns `Option<T>`, the Java
    // facade typically returns `@Nullable T` (via `.orElse(null)`).  Bare-result
    // is_empty/not_empty assertions must use `assertNull/assertNotNull` rather than
    // calling `.isEmpty()` on the nullable reference, which is undefined for record
    // types (mirrors the Kotlin / Zig codegen behaviour).
    let effective_result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

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
                        // Build the typed POJO via `JsonUtil.fromJson(json, Type.class)`.
                        // The Java backend centralizes JSON deserialization in JsonUtil rather
                        // than per-DTO static methods.  Java uses snake_case wire format
                        // (matches Rust's serde default), so pass through fixture keys as-is.
                        let normalized = super::transform_json_keys_for_language(val, "snake_case");
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let escaped = escape_java(&json_str);
                        let var_name = &arg.name;
                        builder_expressions.push_str(&format!(
                            "        var {var_name} = JsonUtil.fromJson(\"{escaped}\", {opts_type}.class);\n",
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

    let adapter = adapters.iter().find(|a| a.name == call_config.function.as_str());
    let adapter_request_type: Option<String> = adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // Determine if this is a streaming adapter.
    let is_streaming_adapter =
        adapter.is_some_and(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));

    // When a non-streaming adapter with owner_type is present, filter out handle-type args
    // since the facade method doesn't take them separately (the handle is
    // encapsulated in the adapter).
    let filtered_args: Vec<_> = if adapter.is_some_and(|a| a.owner_type.is_some()) && !is_streaming_adapter {
        args.iter().filter(|arg| arg.arg_type != "handle").cloned().collect()
    } else {
        args.to_vec()
    };

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`engine.crawlStream(req)`), not as static facade methods — the
    // Java facade deliberately emits no static streaming methods. Capture the owner
    // handle variable so the call is rendered as an instance-method invocation.
    let streaming_owner_handle: Option<String> =
        if is_streaming_adapter && adapter.is_some_and(|a| a.owner_type.is_some()) {
            filtered_args
                .iter()
                .find(|a| a.arg_type == "handle")
                .map(|a| a.name.clone())
        } else {
            None
        };

    let mut teardown_block = String::new();
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        &filtered_args,
        JavaArgsContext {
            class_name,
            options_type: effective_options_type,
            fixture,
            adapter_request_type: adapter_request_type.as_deref(),
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
            config,
            type_defs,
            teardown_block: &mut teardown_block,
        },
    );

    // Per-language `extra_args` from call overrides — verbatim trailing
    // expressions appended after the configured args (e.g. `null` for an
    // optional trailing parameter the fixture cannot supply). Mirrors the
    // TypeScript and C# implementations.
    let extra_args_slice: &[String] = recipe.extra_args;

    let mut final_args = args_str;
    if let Some(visitor_spec) = &fixture.visitor {
        let visitor_var = build_java_visitor(&mut setup_lines, visitor_spec, class_name);
        if let Some(binding) = java_visitor_binding(config, effective_options_type) {
            final_args = apply_java_visitor_arg(&mut setup_lines, &final_args, args, &visitor_var, &binding);
        }
    }

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
    // For assertions, use assert_enum_fields from the call override to get field->type mappings.
    // Build a HashMap that merges both for assertion handling.
    let assert_enum_types: std::collections::HashMap<String, String> = if let Some(co) = call_overrides {
        co.assert_enum_fields.clone()
    } else {
        std::collections::HashMap::new()
    };

    // Keep the old effective_enum_fields as a HashSet for backward compatibility with other code paths.
    let mut effective_enum_fields: std::collections::HashSet<String> = enum_fields.clone();
    if let Some(co) = call_overrides {
        for k in co.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }

    // Streaming detection (call-level `streaming` opt-out is honored). Computed
    // here so `render_assertion` can suppress the streaming-virtual-field path
    // for non-streaming fixtures whose real result struct has a literal `chunks`
    // field that would otherwise collide with the virtual aggregator name.
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let streaming_item_type =
        crate::e2e::codegen::recipe::streaming_item_type(call_config, adapters, &[call_config.function.as_str()]);

    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            class_name,
            field_resolver,
            effective_result_is_simple,
            effective_result_is_bytes,
            effective_result_is_option,
            is_streaming,
            streaming_item_type,
            &effective_enum_fields,
            &assert_enum_types,
        );
    }

    let throws_clause = " throws Exception";

    // When client_factory is set, instantiate a client and dispatch the call as
    // a method on the client; otherwise call the static helper on `class_name`.
    let (client_setup_lines, call_target) = if let Some(factory) = client_factory.as_deref() {
        let factory_name = factory.to_lower_camel_case();
        let fixture_id = &fixture.id;
        let mut setup: Vec<String> = Vec::new();
        let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
        let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            setup.push(format!("String apiKey = System.getenv(\"{var}\");"));
            setup.push(format!(
                "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String baseUrl = (apiKey != null && !apiKey.isEmpty()) ? null : (mockServerUrl != null ? mockServerUrl + \"/fixtures/{fixture_id}\" : \"http://localhost:8000/fixtures/{fixture_id}\");"
            ));
            setup.push(format!(
                "System.out.println(\"{fixture_id}: \" + (baseUrl == null ? \"using real API ({var} is set)\" : \"using mock server ({var} not set)\"));"
            ));
            setup.push(format!(
                "var client = {class_name}.{factory_name}(baseUrl == null ? apiKey : \"test-key\", baseUrl, null, null, null);"
            ));
        } else if has_mock {
            if fixture.has_host_root_route() {
                setup.push(format!(
                    "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String defaultUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\"; String mockUrl = System.getProperty(\"mockServer.{fixture_id}\", defaultUrl);"
                ));
            } else {
                setup.push(format!(
                    "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String mockUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\";"
                ));
            }
            setup.push(format!(
                "var client = {class_name}.{factory_name}(\"test-key\", mockUrl, null, null, null);"
            ));
        } else if let Some(api_key_var) = api_key_var {
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

    let call_expr = if let Some(ref handle_var) = streaming_owner_handle {
        // Instance-method invocation on the owner handle.
        format!("{handle_var}.{function_name}({final_args})")
    } else {
        format!("{call_target}.{function_name}({final_args})")
    };

    // `is_streaming` was computed earlier (before the assertion render loop).
    let collect_snippet = if is_streaming && !expects_error {
        // Derive the item_type from the adapter if present; otherwise use the default.
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet_typed(
            "java",
            result_var,
            "chunks",
            streaming_item_type,
        )
        .unwrap_or_default()
    } else {
        String::new()
    };

    let rendered = crate::e2e::template_env::render(
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
            returns_void => call_config.returns_void,
            collect_snippet => collect_snippet,
            assertions_body => assertions_body,
            teardown_block => teardown_block,
        },
    );
    out.push_str(&rendered);
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
struct JavaArgsContext<'a> {
    class_name: &'a str,
    options_type: Option<&'a str>,
    fixture: &'a crate::e2e::fixture::Fixture,
    adapter_request_type: Option<&'a str>,
    owner_handle_is_receiver: bool,
    config: &'a ResolvedCrateConfig,
    type_defs: &'a [crate::core::ir::TypeDef],
    teardown_block: &'a mut String,
}

fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    context: JavaArgsContext<'_>,
) -> (Vec<String>, String) {
    let JavaArgsContext {
        class_name,
        options_type,
        fixture,
        adapter_request_type,
        owner_handle_is_receiver,
        config,
        type_defs,
        teardown_block,
    } = context;
    let fixture_id = &fixture.id;
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                setup_lines.push(format!(
                    "String {} = System.getProperty(\"mockServer.{fixture_id}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\");",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "String {} = System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\";",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}Req", arg.name);
                setup_lines.push(format!("var {req_var} = new {req_type}({});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // List<String> of URLs: each element is either a bare path (`/seed1`) —
            // prefixed with the per-fixture mock-server URL at runtime — or an absolute
            // URL kept as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>`
            // env var first, then `MOCK_SERVER_URL/fixtures/<id>`. Emitted as a typed
            // `java.util.List<String>` so it matches the binding signature.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_java(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            // Per-fixture mock-server URL resolution order:
            //   1. System.getProperty("mockServer.<fixture_id>") — populated by
            //      MockServerListener from the mock-server's MOCK_SERVERS=
            //      announcement (preferred for host-root-route fixtures).
            //   2. System.getenv("MOCK_SERVER_<FIXTURE_ID>") — explicit env override
            //      for CI / external harnesses.
            //   3. System.getenv("MOCK_SERVER_URL") + "/fixtures/<fixture_id>" —
            //      fallback to the shared-route URL for fixtures without host-root
            //      routes.
            // Previous code skipped (1), so any fixture with per-fixture host-root
            // routes hit /fixtures/<id>/<path> on the shared host — which mock-server
            // doesn't serve — and returned 404 for every batch URL.
            setup_lines.push(format!(
                "String {name}Base = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", (System.getProperty(\"mockServerUrl\") != null ? System.getProperty(\"mockServerUrl\") : (System.getenv(\"MOCK_SERVER_URL\") != null ? System.getenv(\"MOCK_SERVER_URL\") : \"http://localhost:8000\")) + \"/fixtures/{fixture_id}\"));"
            ));
            setup_lines.push(format!(
                "java.util.List<String> {name} = java.util.Arrays.stream(new String[]{{{paths_literal}}}).map(p -> p.startsWith(\"http\") ? p : {name}Base + p).collect(java.util.stream.Collectors.toList());"
            ));
            // Wrap in adapter request type if present (e.g., BatchCrawlStreamRequest).
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}Req", arg.name);
                setup_lines.push(format!("var {req_var} = new {req_type}({});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(name.clone());
            }
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
                if let Some(config_type) = resolve_handle_config_type(arg, options_type, type_defs) {
                    setup_lines.push(format!(
                        "var {name}Config = MAPPER.readValue(\"{}\", {config_type}.class);",
                        escape_java(&json_str),
                    ));
                    setup_lines.push(format!(
                        "var {} = {class_name}.{constructor_name}({name}Config);",
                        arg.name,
                        name = name,
                    ));
                } else {
                    setup_lines.push(format!("var {} = {class_name}.{constructor_name}(null);", arg.name,));
                }
            }
            // For streaming owner_type adapters the handle is the instance-method
            // receiver, not a positional argument — emit its construction but omit
            // it from the call's argument list.
            if owner_handle_is_receiver {
                continue;
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    // Filter to only methods that appear in the Java trait-bridge interface.
                    // Async methods (extract_bytes, extract_file) are handled by the FFI bridge internally.
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| {
                            t.methods
                                .iter()
                                .filter(|m| {
                                    // Skip methods in the ffi_skip_methods list
                                    if trait_bridge.ffi_skip_methods.contains(&m.name) {
                                        return false;
                                    }

                                    // Skip only known non-trait methods not in Java trait-bridge interfaces
                                    match m.name.as_str() {
                                        "description" | "author" => return false,
                                        _ => {}
                                    }

                                    // As of the trait method extraction fix, methods returning excluded types
                                    // are now kept in the interface with type substitution.
                                    // Methods like extract_bytes/extract_file and backend_type are now included.
                                    true
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // Include super-trait methods so the stub can implement them.
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                if !methods.iter().any(|m| m.name == method.name)
                                    && !trait_bridge.ffi_skip_methods.contains(&method.name)
                                    && !matches!(method.name.as_str(), "description" | "author")
                                {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    let excluded_named =
                        crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(config, type_defs, &methods);

                    // Do NOT filter out methods that return excluded types. As of the trait method extraction
                    // fix, trait methods with excluded type signatures are now kept in the interface with type
                    // substitution (excluded types become String). The trait-bridge interface properly handles
                    // these via emit_test_backend_with_context, which uses excluded_named to substitute types.

                    // Call java::emit_test_backend_with_context so stubs handle excluded types correctly.
                    let emission = emit_test_backend_with_context(
                        trait_bridge,
                        &methods,
                        fixture,
                        &config.java_package(),
                        &excluded_named,
                        class_name,
                    );
                    setup_lines.push(emission.setup_block);
                    parts.push(emission.arg_expr);
                    teardown_block.push_str(&emission.teardown_block);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("java");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
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
                    if v.is_array() {
                        if let Some(elem_type) = &arg.element_type {
                            // For complex types, deserialize each array element via JsonUtil.
                            if !is_numeric_type_hint(elem_type) {
                                parts.push(emit_java_object_array(v, elem_type));
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
                // bytes args carry a relative file path (e.g. "docx/fake.docx") that the
                // e2e harness resolves against test_documents/. Read the file at runtime,
                // not the raw path string's UTF-8 bytes.
                if arg.arg_type == "bytes" {
                    let val = json_to_java(v);
                    parts.push(format!(
                        "java.nio.file.Files.readAllBytes(java.nio.file.Path.of({val}))"
                    ));
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
    result_is_option: bool,
    is_streaming: bool,
    streaming_item_type: Option<&str>,
    enum_fields: &std::collections::HashSet<String>,
    assert_enum_types: &std::collections::HashMap<String, String>,
) {
    // Bare-result is_empty / not_empty on Option<T> returns: the Java facade exposes
    // these as `@Nullable T` (via `.orElse(null)`) rather than `Optional<T>`, so the
    // template's `.isEmpty()` call would not compile for record types. Emit a
    // null-check instead — mirrors the kotlin / zig codegen behaviour.
    let bare_field = assertion.field.as_deref().is_none_or(str::is_empty);
    if result_is_option && bare_field {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                out.push_str(&format!(
                    "        assertNull({result_var}, \"expected empty value\");\n"
                ));
                return;
            }
            "not_empty" => {
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-empty value\");\n"
                ));
                return;
            }
            _ => {}
        }
    }

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
            "not_error" => {
                // Use the statically-imported assertion (org.junit.jupiter.api.Assertions.*)
                // so we don't need a separate FQN import of the `Assertions` class.
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-null byte[] response\");\n"
                ));
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
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.content() != null && !c.content().isBlank())"
                );
                out.push_str(&crate::e2e::template_env::render(
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
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.metadata().headingContext() != null)"
                );
                out.push_str(&crate::e2e::template_env::render(
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
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.embedding() != null && !c.embedding().isEmpty())"
                );
                out.push_str(&crate::e2e::template_env::render(
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
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().findFirst().map(c -> c.metadata().headingContext() != null).orElse(false)"
                );
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                        out.push_str(&crate::e2e::template_env::render(
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

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    // Gate on `is_streaming` so non-streaming fixtures (e.g. consumers whose real
    // result struct has a literal `chunks` field) don't divert into the virtual
    // accessor path — they should fall through to the normal field resolver.
    if let Some(f) = &assertion.field {
        if is_streaming && !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                    f,
                    "java",
                    "chunks",
                    None,
                    streaming_item_type,
                )
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr}.size() >= {n}, \"expected >= {n} chunks\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertEquals({n}, {expr}.size());\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_java(s);
                            format!("        assertEquals(\"{escaped}\", {expr});\n")
                        } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertEquals({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\");\n"),
                    "is_empty" => format!("        assertTrue({expr}.isEmpty(), \"expected empty\");\n"),
                    "is_true" => format!("        assertTrue({expr}, \"expected true\");\n"),
                    "is_false" => format!("        assertFalse({expr}, \"expected false\");\n"),
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} > {n}, \"expected > {n}\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} >= {n}, \"expected >= {n}\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_java(s);
                            format!(
                                "        assertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");\n"
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
            out.push_str(&crate::e2e::template_env::render(
                "java/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // Determine if this field maps to a sealed-interface type declared in
    // `assert_enum_types`.  When `Some`, the value is the type name (e.g.
    // "FormatMetadata") and the corresponding `{TypeName}Display` helper will
    // be used to produce the display string for assertions.
    let sealed_display_type: Option<String> = assertion.field.as_deref().and_then(|f| {
        let resolved = field_resolver.resolve(f);
        assert_enum_types
            .get(f)
            .or_else(|| assert_enum_types.get(resolved))
            .cloned()
    });
    let is_sealed_display_field = sealed_display_type.is_some();

    // Determine if this field is an enum type (no `.contains()` on enums in Java).
    // Check both the raw fixture field path and the resolved (aliased) path so that
    // `fields_enum` entries can use either form (e.g., `"assets[].category"` or the
    // resolved `"assets[].asset_category"`).
    // NOTE: Sealed-interface types (those in assert_enum_types) are not Java enums
    // and do not have a .getValue() method — exclude them from enum field treatment.
    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let in_enum_fields = enum_fields.get(f).is_some() || enum_fields.get(resolved).is_some();
        in_enum_fields && !is_sealed_display_field
    });

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
                            _ => {
                                // `field_is_enum` already excludes sealed-interface types
                                // (is_sealed_display_field), so any remaining enum type
                                // has .getValue() available.
                                format!("{optional_expr}.map(v -> v.getValue()).orElse(\"\")")
                            }
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
                            // For numeric comparisons on Optional<Long/Integer> fields, coerce
                            // the boxed numeric type to `long` via Number::longValue so the same
                            // code path compiles for both `Optional<Integer>` (e.g. mapped from
                            // Rust `Option<u32>`) and `Optional<Long>` fields.  Using a bare
                            // `.orElse(0L)` would fail for `Optional<Integer>` because the
                            // fallback type would not match the element type.
                            "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                                if field_resolver.is_array(resolved) {
                                    format!("{optional_expr}.orElse(java.util.List.of())")
                                } else {
                                    format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                }
                            }
                            // For equals on Optional fields, determine fallback based on whether value is numeric.
                            // If the fixture value is a number, coerce via Number::longValue so the
                            // comparison compiles for both Optional<Integer> and Optional<Long>.
                            // Sealed-display fields are handled via the {TypeName}Display helper in
                            // string_expr — keep as Optional here so the helper receives the unwrapped value.
                            "equals" => {
                                if is_sealed_display_field {
                                    // Sealed-interface Optional: keep, will be handled by string_expr path
                                    optional_expr
                                } else if let Some(expected) = &assertion.value {
                                    if expected.is_number() {
                                        format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                    } else {
                                        // `.map(Objects::toString)` collapses Optional<T> to
                                        // Optional<String> before `.orElse("")`, so the result
                                        // is unambiguously a String even when T is `Object`
                                        // (which is the Java mapping for free-form JSON values
                                        // like `Option<serde_json::Value>` — javac otherwise
                                        // infers LUB(Object, String) = Object and breaks
                                        // String-only method calls downstream like .contains()).
                                        format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                    }
                                } else {
                                    format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                }
                            }
                            _ if field_resolver.is_array(resolved) => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            _ => format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")"),
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
    // Sealed-interface types (is_sealed_display_field) use a pattern-match helper instead.
    let string_expr = if field_is_enum && !field_expr.contains(".map(v -> v.getValue())") {
        format!("{field_expr}.getValue()")
    } else if let Some(ref stype) = sealed_display_type {
        // Sealed-interface type: convert via a generated `{TypeName}Display.toDisplayString`
        // helper that pattern-matches over all variants from the IR.
        // For Optional<T>, unwrap with orElse(null) so the helper can handle null safely.
        let inner_expr = if field_expr.contains("Optional.ofNullable") {
            format!("{field_expr}.orElse(null)")
        } else {
            field_expr.clone()
        };
        format!("{stype}Display.toDisplayString({inner_expr})")
    } else {
        field_expr.clone()
    };

    // Pre-compute context for template
    let assertion_type = assertion.assertion_type.as_str();
    let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let is_numeric_val = assertion.value.as_ref().is_some_and(|v| v.is_number());

    // values_java is consumed by `contains`, `contains_all`, `contains_any`, and
    // `not_contains` loops. Fall back to wrapping the singular `value` so single-entry
    // fixtures still emit one assertion call per value instead of an empty loop.
    let values_java: Vec<String> = assertion
        .values
        .as_ref()
        .map(|values| values.iter().map(json_to_java).collect::<Vec<_>>())
        .or_else(|| assertion.value.as_ref().map(|v| vec![json_to_java(v)]))
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

    let rendered = crate::e2e::template_env::render(
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

/// Build a Java call expression for a `method_result` assertion on a sample_language Tree.
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

/// Emit a Java list of deserialized objects via JsonUtil.
/// E.g., `[{"type": "click", ...}, ...]` becomes `java.util.Arrays.asList(JsonUtil.fromJson(..., PageAction.class), ...)`.
fn emit_java_object_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        if items.is_empty() {
            return "java.util.List.of()".to_string();
        }
        let item_strs: Vec<String> = items
            .iter()
            .map(|item| {
                let json_str = serde_json::to_string(item).unwrap_or_default();
                let escaped = escape_java(&json_str);
                format!("JsonUtil.fromJson(\"{escaped}\", {elem_type}.class)")
            })
            .collect();
        format!("java.util.Arrays.asList({})", item_strs.join(", "))
    } else {
        "java.util.List.of()".to_string()
    }
}

/// Convert a `serde_json::Value` to a Java literal string.
fn json_to_java(value: &serde_json::Value) -> String {
    json_to_java_typed(value, None)
}

/// Convert a JSON value to a Java literal, optionally overriding number type for array elements.
/// `element_type` controls how numeric array elements are emitted: "f32" -> `1.0f`, otherwise `1.0d`.
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
                // by default since most options builder fields are Optional. Calls that
                // use primitive builder fields can opt into bare values by setting
                // `nested_types_optional = false`.
                let camel_key = key.to_lower_camel_case();
                let is_plain_field = matches!(camel_key.as_str(), "listIndentWidth" | "wrapWidth");
                let is_primitive_builder = !nested_types_optional;

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
                // Top-level config builders usually declare nested record fields as
                // `Optional<T>`. Calls with non-optional nested config builders can opt
                // into passing the bare builder result.
                let is_primitive_builder = !nested_types_optional;
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
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
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

#[derive(Debug, Clone)]
struct JavaVisitorBinding {
    options_type: String,
    options_field: String,
}

fn java_visitor_binding(
    config: &ResolvedCrateConfig,
    fallback_options_type: Option<&str>,
) -> Option<JavaVisitorBinding> {
    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.options_type.is_some() && bridge.resolved_options_field().is_some())?;
    Some(JavaVisitorBinding {
        options_type: fallback_options_type
            .or(bridge.options_type.as_deref())
            .map(str::to_string)?,
        options_field: bridge.resolved_options_field()?.to_string(),
    })
}

fn apply_java_visitor_arg(
    setup_lines: &mut Vec<String>,
    args_str: &str,
    args: &[crate::e2e::config::ArgMapping],
    visitor_var: &str,
    binding: &JavaVisitorBinding,
) -> String {
    let wither = format!("with{}", binding.options_field.to_upper_camel_case());
    if let Some(options_arg) = args
        .iter()
        .find(|arg| arg.arg_type == "json_object" && args_str.split(", ").any(|part| part == arg.name))
    {
        setup_lines.push(format!(
            "{} = {}.{}({});",
            options_arg.name, options_arg.name, wither, visitor_var
        ));
        return args_str.to_string();
    }

    let options_expr = format!("new {}().{}({})", binding.options_type, wither, visitor_var);
    if args_str.is_empty() {
        options_expr
    } else if let Some(stripped) = args_str.strip_suffix(", null") {
        format!("{stripped}, {options_expr}")
    } else {
        format!("{args_str}, {options_expr}")
    }
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
        CallbackAction::CustomTemplate { template, .. } => {
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

    let rendered = crate::e2e::template_env::render(
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

/// Map a TypeRef to its Java type with fully-qualified names for use in test stubs.
/// This variant ensures all types are qualified (e.g., `java.util.List` not `List`).
fn java_type_fqn(ty: &crate::core::ir::TypeRef) -> String {
    use crate::backends::java::type_map::java_type;
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(_) => "Object".to_string(),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "Object".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "java.util.List<Object>".to_string(),
        TypeRef::Vec(_) => {
            // Use JavaBoxedMapper to get boxed inner types, then qualify List
            format!("java.util.{}", java_type(ty).into_owned())
        }
        TypeRef::Map(_, _) => {
            // Use JavaBoxedMapper to get boxed inner types, then qualify Map
            format!("java.util.{}", java_type(ty).into_owned())
        }
        _ => {
            let t = java_type(ty).into_owned();
            match t.as_str() {
                "List" | "ArrayList" => format!("java.util.{}", t),
                "Map" | "HashMap" => format!("java.util.{}", t),
                _ => t,
            }
        }
    }
}

/// Map a TypeRef to its Java stub type with fully-qualified names.
///
/// Named types are qualified with `binding_pkg` (e.g. `dev.example`) which is the
/// actual Java package of the binding, matching what the Panama FFM interface declares.
/// Pass `""` to fall back to unqualified simple names (used by the generic dispatch path).
fn java_stub_type_fqn(ty: &crate::core::ir::TypeRef, binding_pkg: &str) -> String {
    use crate::core::ir::TypeRef;
    let pkg_prefix = if binding_pkg.is_empty() {
        String::new()
    } else {
        format!("{binding_pkg}.")
    };
    match ty {
        TypeRef::Named(name) => {
            // Qualify all named types with the binding package so the generated stub
            // compiles against the actual interface in the binding jar/module.
            format!("{pkg_prefix}{name}")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => format!("{pkg_prefix}{name}"),
            other => java_stub_type_fqn(other, binding_pkg),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) => format!("java.util.List<{pkg_prefix}{name}>"),
            other => format!("java.util.List<{}>", java_stub_type_fqn(other, binding_pkg)),
        },
        TypeRef::Map(k, v) => {
            let key_type = java_stub_type_fqn(k, binding_pkg);
            let val_type = java_stub_type_fqn(v, binding_pkg);
            format!("java.util.Map<{}, {}>", key_type, val_type)
        }
        _ => java_type_fqn(ty),
    }
}

/// Map a TypeRef to its Java stub type with excluded-types context.
///
/// When a Named type is in `excluded_types`, it is substituted with `String`
/// (matching the trait-bridge interface which serializes excluded types to JSON strings).
/// Otherwise behaves like `java_stub_type_fqn`.
/// Box a Java type for use in generic parameters (List<T>, Map<K,V>).
/// Primitive types like `float` become `Float`, but already-boxed and complex types pass through.
fn box_java_type_for_generic(ty: &str) -> String {
    match ty {
        "boolean" => "Boolean".to_string(),
        "byte" => "Byte".to_string(),
        "short" => "Short".to_string(),
        "int" => "Integer".to_string(),
        "long" => "Long".to_string(),
        "float" => "Float".to_string(),
        "double" => "Double".to_string(),
        "char" => "Character".to_string(),
        other => other.to_string(),
    }
}

fn java_stub_type_with_context(
    ty: &crate::core::ir::TypeRef,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            "String".to_string()
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
                "String".to_string()
            }
            other => java_stub_type_with_context(other, binding_pkg, excluded_types),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
                "java.util.List<String>".to_string()
            }
            other => {
                let inner_type = java_stub_type_with_context(other, binding_pkg, excluded_types);
                // Box primitives for use in generic parameters
                let boxed_inner = box_java_type_for_generic(&inner_type);
                format!("java.util.List<{boxed_inner}>")
            }
        },
        TypeRef::Map(k, v) => {
            let key_type = java_stub_type_with_context(k, binding_pkg, excluded_types);
            let val_type = java_stub_type_with_context(v, binding_pkg, excluded_types);
            // Box primitives for use in generic parameters
            let boxed_key = box_java_type_for_generic(&key_type);
            let boxed_val = box_java_type_for_generic(&val_type);
            format!("java.util.Map<{}, {}>", boxed_key, boxed_val)
        }
        _ => java_stub_type_fqn(ty, binding_pkg),
    }
}

/// Boxed version of java_stub_type_with_context for use as a CompletableFuture generic parameter.
#[allow(dead_code)]
fn java_boxed_stub_type_with_context(
    ty: &crate::core::ir::TypeRef,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Unit => "Void".to_string(),
        _ => {
            let t = java_stub_type_with_context(ty, binding_pkg, excluded_types);
            // Box primitives for use as generic type parameters.
            match t.as_str() {
                "boolean" => "Boolean".to_string(),
                "byte" => "Byte".to_string(),
                "short" => "Short".to_string(),
                "int" => "Integer".to_string(),
                "long" => "Long".to_string(),
                "float" => "Float".to_string(),
                "double" => "Double".to_string(),
                "byte[]" => "byte[]".to_string(), // byte[] stays as-is (already boxed in Java)
                _ => t,
            }
        }
    }
}

/// Return the default value for a type, substituting excluded types with `""`.
fn java_stub_default_with_context(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) -> String {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            "\"\"".to_string()
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !excluded_types.is_empty() && excluded_types.contains(n.as_str())) => {
            "\"\"".to_string()
        }
        // For Named types that are NOT excluded, return null instead of trying to instantiate.
        // Complex types like ExtractionResult don't have no-arg constructors, and stub
        // methods are only used for testing trait bridge registration, not for exercising
        // the actual functionality. Returning null is safe here.
        TypeRef::Named(_) => "null".to_string(),
        _ => defaults.emit_default(ty),
    }
}

/// Emit a single Java stub method with excluded-types context.
///
/// Like `emit_java_stub_method` but with excluded_types substitution.
/// Excluded types are rendered as `String` in signatures and default to `""`.
fn emit_java_stub_method_with_context(
    out: &mut String,
    method_java: &str,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) {
    use std::fmt::Write as _;

    let ret_java = java_stub_type_with_context(&method.return_type, binding_pkg, excluded_types);
    let default_val = java_stub_default_with_context(&method.return_type, excluded_types, defaults);

    // Use java_stub_type_with_context for all parameter types to handle excluded types
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            format!(
                "{} {}",
                java_stub_type_with_context(&p.ty, binding_pkg, excluded_types),
                p.name.to_lower_camel_case()
            )
        })
        .collect();
    let params_str = params.join(", ");

    let _ = writeln!(out, "    @Override");
    // E2e test stubs must match the trait bridge interface signatures exactly.
    // The interface declares sync methods (not wrapped in CompletableFuture),
    // even if the Rust trait method is async. The trait bridge handles async
    // internally; test stubs just implement the interface signature.
    if ret_java == "void" {
        let _ = writeln!(out, "    public void {method_java}({params_str}) {{}}");
    } else {
        let _ = writeln!(out, "    public {ret_java} {method_java}({params_str}) {{");
        let _ = writeln!(out, "        return {default_val};");
        let _ = writeln!(out, "    }}");
    }
}

/// Emit a Java test backend stub class for a trait bridge.
///
/// Generates a class implementing `I{TraitName}` (the Panama FFM interface). Required
/// methods are overridden with `CompletableFuture.completedFuture(default)` for async
/// signatures or the direct default value for sync. The `name()` method is emitted when
/// a Plugin super-trait is configured.
///
/// `binding_pkg` is the Java package of the binding (e.g. `dev.example`). It is used
/// to fully-qualify named types in method signatures and the interface name. Pass `""`
/// when calling from the generic dispatch path (types will be unqualified).
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    binding_pkg: &str,
) -> super::TestBackendEmission {
    emit_test_backend_with_context(trait_bridge, methods, fixture, binding_pkg, &Default::default(), "")
}

/// Like `emit_test_backend` but with excluded_types context.
///
/// Excluded types are substituted with `String` in method signatures and default to `""`.
/// This matches how the trait-bridge interface serializes binding-excluded types to JSON strings.
///
/// `binding_class` is the unqualified class name used for static teardown calls
/// (e.g. `unregister_<trait>`). When empty, teardown is omitted.
pub fn emit_test_backend_with_context(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
    binding_class: &str,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::escape_java;
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Java interface follows the I{TraitName} convention from the Panama FFM bridge.
    // Use fully-qualified name to avoid "cannot find symbol" errors in test compilation.
    let interface_name = if binding_pkg.is_empty() {
        format!("I{}", trait_bridge.trait_name)
    } else {
        format!("{binding_pkg}.I{}", trait_bridge.trait_name)
    };

    let plugin_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let backend_name = plugin_name.clone();

    let defaults = language_defaults("java");

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name} implements {interface_name} {{");

    // Super-trait methods — driven from IR, no names hardcoded.
    // The `name` method returns the fixture's plugin name; all others use defaults.
    // Method names must match the interface exactly (snake_case).
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_java = &method.name; // Keep snake_case to match interface
            if method.name == "name" {
                let _ = writeln!(setup, "    @Override");
                let _ = writeln!(
                    setup,
                    "    public String {method_java}() {{ return \"{plugin_name}\"; }}"
                );
            } else {
                emit_java_stub_method_with_context(
                    &mut setup,
                    method_java,
                    method,
                    &*defaults,
                    binding_pkg,
                    excluded_types,
                );
            }
        }
    }

    // All non-super-trait methods (including those with default impls).
    // Java interfaces require all abstract methods to be implemented, even if
    // Rust traits provide default implementations.
    // Method names must match the interface exactly (snake_case).
    for method in methods {
        // Skip super-trait methods already emitted above.
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        let method_java = &method.name; // Keep snake_case to match interface
        if method.name == "name" {
            let _ = writeln!(setup, "    @Override");
            let _ = writeln!(
                setup,
                "    public String {method_java}() {{ return \"{plugin_name}\"; }}"
            );
        } else {
            emit_java_stub_method_with_context(
                &mut setup,
                method_java,
                method,
                &*defaults,
                binding_pkg,
                excluded_types,
            );
        }
    }

    let _ = writeln!(setup, "}}");

    // Java test runner (JUnit) runs each test in the same process, so registering a
    // test backend leaks into later tests. Emit `<BindingClass>.unregister_<trait>("backend_name")`
    // after the call+assertions to drain the test backend from the global registry.
    let teardown_block = if binding_class.is_empty() {
        String::new()
    } else {
        trait_bridge
            .unregister_fn
            .as_deref()
            .map(|unregister_fn| {
                let escaped = escape_java(&backend_name);
                let camel_case_fn = unregister_fn.to_lower_camel_case();
                format!("        {binding_class}.{camel_case_fn}(\"{escaped}\");\n")
            })
            .unwrap_or_default()
    };

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr: format!("new {class_name}()"),
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Extract a backend name string from the fixture input JSON.
///
/// Searches the top-level input object for the first string value at any depth
/// under keys commonly used for names (`name`, or the first string field found).
/// Falls back to the fixture id when no string is found.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    // Walk the top-level object, then one level deeper, looking for "name".
    if let Some(obj) = input.as_object() {
        // Direct "name" key.
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        // One level deeper in any nested object.
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        // First string value at the top level.
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        }
    }

    /// Verify that no sample-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn java_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        // With empty binding_pkg (generic dispatch path): interface is unqualified.
        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        assert!(
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded 'dev.sample_crate', got:\n{output}"
        );
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "class name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("implements ITestTrait"),
            "class must implement interface with binding_pkg prefix, got:\n{output}"
        );
        assert!(
            output.contains("process_item"),
            "required method must be emitted in snake_case to match interface, got:\n{output}"
        );
    }

    /// Verify that when `binding_pkg` is provided (e.g. `dev.example`), the interface
    /// name and named types in method signatures are fully-qualified with that package.
    #[test]
    fn java_stub_uses_binding_pkg_for_interface_and_type_qualification() {
        let bridge = make_trait_bridge("DocumentExtractor");
        let method = MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![],
            return_type: TypeRef::Named("OperationOutput".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: Some("DocumentExtractor".to_string()),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let methods = [&method];
        let fixture = make_fixture("extract_bytes_test");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "dev.example");
        let output = &emission.setup_block;

        // Interface must be qualified with the binding package.
        assert!(
            output.contains("implements dev.example.IDocumentExtractor"),
            "class must implement dev.example.IDocumentExtractor, got:\n{output}"
        );
        // Named type must be qualified with the binding package.
        assert!(
            output.contains("dev.example.OperationOutput"),
            "return type must use dev.example.OperationOutput, got:\n{output}"
        );
        // Must NOT contain old hardcoded dev.sample_crate.
        assert!(
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded dev.sample_crate, got:\n{output}"
        );
    }

    /// Test that plugin name is correctly extracted from nested input object.
    #[test]
    fn java_stub_plugin_name_extracted_from_input_name_field() {
        let bridge = make_trait_bridge("DocumentExtractor");
        let mut name_method = make_method("name", true);
        name_method.trait_source = Some("Plugin".to_string());
        let methods = [&name_method];
        let fixture = Fixture {
            id: "register_document_extractor_trait_bridge".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({
                "extractor": {
                    "type": "test",
                    "name": "test-extractor"
                }
            }),
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        };

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = &emission.setup_block;

        // The name() method must return the value from input.extractor.name
        assert!(
            output.contains("public String name() { return \"test-extractor\"; }"),
            "name() method must return extracted name 'test-extractor', got:\n{output}"
        );
    }

    /// Test that stub method signatures use fully-qualified names for domain types
    /// when the actual binding package is unknown (empty string fallback).
    #[test]
    fn java_stub_method_uses_fqn_for_domain_types_no_pkg() {
        let bridge = make_trait_bridge("DocumentExtractor");
        // Method returning a domain type
        let method = MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![],
            return_type: TypeRef::Named("OperationOutput".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: Some("DocumentExtractor".to_string()),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let methods = [&method];
        let fixture = make_fixture("extract_bytes_test");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = &emission.setup_block;

        // With empty binding_pkg, named types are unqualified.
        // Method names must use snake_case to match the interface.
        assert!(
            output.contains("public OperationOutput extract_bytes"),
            "return type must use OperationOutput (unqualified, empty pkg) with snake_case method name, got:\n{output}"
        );
        // Must NOT contain hardcoded dev.sample_crate.
        assert!(
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded dev.sample_crate, got:\n{output}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig, SelectWhen};
    use crate::e2e::fixture::Fixture;
    use std::collections::HashMap;

    fn make_fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test fixture".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        }
    }

    /// Test that resolve_call_for_fixture correctly routes to batchScrape
    /// when input has batch_urls and select_when condition matches.
    #[test]
    fn test_java_select_when_routes_to_batch_scrape() {
        let mut calls = HashMap::new();
        calls.insert(
            "batch_scrape".to_string(),
            CallConfig {
                function: "batchScrape".to_string(),
                module: "com.example.sample_stream".to_string(),
                select_when: Some(SelectWhen {
                    input_has: Some("batch_urls".to_string()),
                    ..Default::default()
                }),
                ..CallConfig::default()
            },
        );

        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "scrape".to_string(),
                module: "com.example.sample_stream".to_string(),
                ..CallConfig::default()
            },
            calls,
            ..E2eConfig::default()
        };

        // Fixture with batch_urls but no explicit call field should route to batch_scrape
        let fixture = make_fixture_with_input("batch_empty_urls", serde_json::json!({ "batch_urls": [] }));

        let resolved_call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        assert_eq!(resolved_call.function, "batchScrape");

        // Fixture without batch_urls should fall back to default scrape
        let fixture_no_batch =
            make_fixture_with_input("simple_scrape", serde_json::json!({ "url": "https://example.com" }));
        let resolved_default = e2e_config.resolve_call_for_fixture(
            fixture_no_batch.call.as_deref(),
            &fixture_no_batch.id,
            &fixture_no_batch.resolved_category(),
            &fixture_no_batch.tags,
            &fixture_no_batch.input,
        );
        assert_eq!(resolved_default.function, "scrape");
    }

    #[test]
    fn handle_config_deserialization_uses_resolved_options_type() {
        let args = vec![ArgMapping {
            name: "session".to_string(),
            field: "input.config".to_string(),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            trait_name: None,
        }];
        let fixture = make_fixture_with_input("session_fixture", serde_json::json!({ "config": { "limit": 3 } }));
        let mut teardown = String::new();
        let (setup, args_str) = build_args_and_setup(
            &fixture.input,
            &args,
            JavaArgsContext {
                class_name: "Sample",
                options_type: Some("SessionConfig"),
                fixture: &fixture,
                adapter_request_type: None,
                owner_handle_is_receiver: false,
                config: &ResolvedCrateConfig::default(),
                type_defs: &[],
                teardown_block: &mut teardown,
            },
        );

        let rendered = setup.join("\n");
        assert_eq!(args_str, "session");
        assert!(rendered.contains("MAPPER.readValue(\"{\\\"limit\\\":3}\", SessionConfig.class)"));
        assert!(rendered.contains("Sample.createSession(sessionConfig)"));
        assert!(!rendered.contains("CrawlConfig"));
    }

    #[test]
    fn java_visitor_arg_uses_trait_bridge_options_metadata() {
        use crate::core::config::{BridgeBinding, TraitBridgeConfig};

        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Renderer".to_string(),
                type_alias: Some("RenderHandle".to_string()),
                param_name: Some("renderer".to_string()),
                bind_via: BridgeBinding::OptionsField,
                options_type: Some("RenderOptions".to_string()),
                options_field: Some("callback".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let binding = java_visitor_binding(&config, None).expect("visitor binding");
        assert_eq!(binding.options_type, "RenderOptions");
        assert_eq!(binding.options_field, "callback");

        let args = apply_java_visitor_arg(&mut Vec::new(), "html, null", &[], "visitor", &binding);
        assert_eq!(args, "html, new RenderOptions().withCallback(visitor)");
        assert!(!args.contains("ConversionOptions"));
    }
}
