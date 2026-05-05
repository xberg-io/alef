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
use std::fmt::Write as FmtWrite;
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
        let pkg_version = config.resolved_version().unwrap_or_else(|| "0.1.0".to_string());

        // Generate pom.xml.
        files.push(GeneratedFile {
            path: output_base.join("pom.xml"),
            content: render_pom_xml(&pkg_name, &java_group_id, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate test files per category. Path mirrors the configured Java
        // package — `dev.myorg` becomes `dev/myorg`, etc. — so the package
        // declaration in each test file matches its filesystem location.
        let mut test_base = output_base.join("src").join("test").join("java");
        for segment in java_group_id.split('.') {
            test_base = test_base.join(segment);
        }
        let test_base = test_base.join("e2e");

        // Resolve options_type from override.
        let options_type = overrides.and_then(|o| o.options_type.clone());

        // Get Java-specific enum_fields from override (required for correct enum handling).
        let empty_enum_fields = std::collections::HashMap::new();
        let java_enum_fields = overrides.as_ref().map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);

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
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                result_is_simple,
                java_enum_fields,
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
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <groupId>{java_group_id}</groupId>
    <artifactId>{artifact_id}</artifactId>
    <version>0.1.0</version>

    <properties>
        <maven.compiler.source>25</maven.compiler.source>
        <maven.compiler.target>25</maven.compiler.target>
        <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
        <junit.version>{junit}</junit.version>
    </properties>

    <dependencies>
{dep_block}
        <dependency>
            <groupId>com.fasterxml.jackson.core</groupId>
            <artifactId>jackson-databind</artifactId>
            <version>{jackson}</version>
        </dependency>
        <dependency>
            <groupId>com.fasterxml.jackson.datatype</groupId>
            <artifactId>jackson-datatype-jdk8</artifactId>
            <version>{jackson}</version>
        </dependency>
        <dependency>
            <groupId>org.jetbrains</groupId>
            <artifactId>annotations</artifactId>
            <version>24.1.0</version>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>${{junit.version}}</version>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <plugins>
            <plugin>
                <groupId>org.codehaus.mojo</groupId>
                <artifactId>build-helper-maven-plugin</artifactId>
                <version>{build_helper}</version>
                <executions>
                    <execution>
                        <id>add-test-source</id>
                        <phase>generate-test-sources</phase>
                        <goals>
                            <goal>add-test-source</goal>
                        </goals>
                        <configuration>
                            <sources>
                                <source>src/test/java</source>
                            </sources>
                        </configuration>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-surefire-plugin</artifactId>
                <version>{maven_surefire}</version>
                <configuration>
                    <argLine>--enable-preview --enable-native-access=ALL-UNNAMED -Djava.library.path=${{project.basedir}}/../../target/release</argLine>
                    <workingDirectory>${{project.basedir}}/../../test_documents</workingDirectory>
                </configuration>
            </plugin>
        </plugins>
    </build>
</project>
"#,
        junit = tv::maven::JUNIT,
        jackson = tv::maven::JACKSON_E2E,
        build_helper = tv::maven::BUILD_HELPER_MAVEN_PLUGIN,
        maven_surefire = tv::maven::MAVEN_SUREFIRE_PLUGIN_E2E,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    java_group_id: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &std::collections::HashMap<String, String>,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls.  Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    let _ = writeln!(out, "package {java_group_id}.e2e;");
    let _ = writeln!(out);

    // Check if any fixture (with its resolved call) will emit MAPPER usage.
    // Note: we no longer use MAPPER for json_object options (using builder pattern instead).
    // But we still need it for handle args and HTTP fixtures.
    let lang_for_om = "java";
    let _needs_object_mapper_for_options = false;
    // Also need ObjectMapper when a handle arg has a non-null config.
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
        // Detect batch item types used in this fixture
        for arg in &call_cfg.args {
            if let Some(elem_type) = &arg.element_type {
                if elem_type == "BatchBytesItem" || elem_type == "BatchFileItem" {
                    all_options_types.insert(elem_type.clone());
                }
            }
        }
    }

    let _ = writeln!(out, "import org.junit.jupiter.api.Test;");
    let _ = writeln!(out, "import static org.junit.jupiter.api.Assertions.*;");
    if !import_path.is_empty() {
        let _ = writeln!(out, "import {import_path};");
    }
    if needs_object_mapper {
        let _ = writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper;");
        let _ = writeln!(out, "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;");
    }
    // Collect all enum types used in builder expressions across all fixtures.
    let mut enum_types_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Collect nested config types actually referenced in fixture builder expressions
    // (rather than importing all defaults unconditionally, which causes javac errors
    // when a type like ChunkingConfig doesn't exist in the binding's package).
    let mut nested_types_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in fixtures.iter() {
        let call_cfg = e2e_config.resolve_call(f.call.as_deref());
        for arg in &call_cfg.args {
            if arg.arg_type == "json_object" {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                if let Some(val) = f.input.get(field) {
                    if !val.is_null() && !val.is_array() {
                        if let Some(obj) = val.as_object() {
                            collect_enum_and_nested_types(obj, enum_fields, &mut enum_types_used);
                            collect_nested_type_names(obj, nested_types, &mut nested_types_used);
                        }
                    }
                }
            }
        }
    }

    // Import all options types used across fixtures (for builder expressions and MAPPER).
    if !all_options_types.is_empty() {
        let opts_pkg = if !import_path.is_empty() {
            import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("")
        } else {
            ""
        };
        for opts_type in &all_options_types {
            let qualified = if opts_pkg.is_empty() {
                opts_type.clone()
            } else {
                format!("{opts_pkg}.{opts_type}")
            };
            let _ = writeln!(out, "import {qualified};");
        }
    }

    // Import all enum types used in builder expressions
    if !enum_types_used.is_empty() && !import_path.is_empty() {
        let binding_pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        for enum_type in &enum_types_used {
            let _ = writeln!(out, "import {binding_pkg}.{enum_type};");
        }
    }

    // Import only the nested options types that are actually referenced in fixture
    // builder expressions. Using `nested_types_used` (populated above) rather than
    // all `nested_types.values()` avoids javac `cannot find symbol` errors for types
    // like ChunkingConfig that exist in other Kreuzberg bindings but not this one.
    if !nested_types_used.is_empty() && !import_path.is_empty() {
        let binding_pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        for type_name in &nested_types_used {
            let _ = writeln!(out, "import {binding_pkg}.{type_name};");
        }
    }

    // Import CrawlConfig when handle args need JSON deserialization.
    if needs_object_mapper_for_handle && !import_path.is_empty() {
        let pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        let _ = writeln!(out, "import {pkg}.CrawlConfig;");
    }
    // Import visitor types when any fixture uses visitor callbacks.
    let has_visitor_fixtures = fixtures.iter().any(|f| f.visitor.is_some());
    if has_visitor_fixtures && !import_path.is_empty() {
        let binding_pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        if !binding_pkg.is_empty() {
            let _ = writeln!(out, "import {binding_pkg}.Visitor;");
            let _ = writeln!(out, "import {binding_pkg}.NodeContext;");
            let _ = writeln!(out, "import {binding_pkg}.VisitResult;");
        }
    }
    // Import Optional when using builder expressions with optional fields
    if !all_options_types.is_empty() {
        let _ = writeln!(out, "import java.util.Optional;");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "class {test_class_name} {{");

    if needs_object_mapper {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "    private static final ObjectMapper MAPPER = new ObjectMapper().registerModule(new Jdk8Module());"
        );
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
            nested_types,
            nested_types_optional,
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
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
        let _ = writeln!(out, "    @Test");
        if let Some(reason) = skip_reason {
            let escaped_reason = escape_java(reason);
            let _ = writeln!(out, "    void test{fn_name}() {{");
            let _ = writeln!(out, "        // {description}");
            let _ = writeln!(
                out,
                "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"{escaped_reason}\");"
            );
        } else {
            let _ = writeln!(out, "    void test{fn_name}() throws Exception {{");
            let _ = writeln!(out, "        // {description}");
            // Resolve base URL once at the top of every non-skipped test.
            let _ = writeln!(out, "        String baseUrl = System.getenv(\"MOCK_SERVER_URL\");");
            let _ = writeln!(out, "        if (baseUrl == null) baseUrl = \"http://localhost:8080\";");
        }
    }

    /// Emit the closing `}` for a test method.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
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
        let _ = writeln!(
            out,
            "        java.net.URI uri = java.net.URI.create(baseUrl + \"{path}\");"
        );

        let body_publisher = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_java(&json);
            format!("java.net.http.HttpRequest.BodyPublishers.ofString(\"{escaped}\")")
        } else {
            "java.net.http.HttpRequest.BodyPublishers.noBody()".to_string()
        };

        let _ = writeln!(out, "        var builder = java.net.http.HttpRequest.newBuilder(uri)");
        let _ = writeln!(out, "            .method(\"{method}\", {body_publisher});");

        // Content-Type header — only when a body is present.
        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            // Only emit when not already in ctx.headers (avoid duplicate Content-Type).
            if !ctx.headers.keys().any(|k| k.to_lowercase() == "content-type") {
                let _ = writeln!(
                    out,
                    "        builder = builder.header(\"Content-Type\", \"{content_type}\");"
                );
            }
        }

        // Explicit request headers — skip Java-restricted ones.
        for (name, value) in ctx.headers {
            if JAVA_RESTRICTED_HEADERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let escaped_name = escape_java(name);
            let escaped_value = escape_java(value);
            let _ = writeln!(
                out,
                "        builder = builder.header(\"{escaped_name}\", \"{escaped_value}\");"
            );
        }

        // Cookies as a single `Cookie` header.
        if !ctx.cookies.is_empty() {
            let cookie_str: Vec<String> = ctx.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let cookie_header = escape_java(&cookie_str.join("; "));
            let _ = writeln!(
                out,
                "        builder = builder.header(\"Cookie\", \"{cookie_header}\");"
            );
        }

        let response_var = ctx.response_var;
        let _ = writeln!(
            out,
            "        var {response_var} = java.net.http.HttpClient.newHttpClient()"
        );
        let _ = writeln!(
            out,
            "            .send(builder.build(), java.net.http.HttpResponse.BodyHandlers.ofString());"
        );
    }

    /// Emit `assertEquals(status, response.statusCode(), ...)`.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "        assertEquals({status}, {response_var}.statusCode(), \"status code mismatch\");"
        );
    }

    /// Emit a header assertion using `response.headers().firstValue(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_java(name);
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isPresent(), \"header {escaped_name} should be present\");"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").isEmpty(), \"header {escaped_name} should be absent\");"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").matches(\"[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\"), \"header {escaped_name} should be a UUID\");"
                );
            }
            literal => {
                let escaped_value = escape_java(literal);
                let _ = writeln!(
                    out,
                    "        assertTrue({response_var}.headers().firstValue(\"{escaped_name}\").orElse(\"\").contains(\"{escaped_value}\"), \"header {escaped_name} mismatch\");"
                );
            }
        }
    }

    /// Emit a JSON body equality assertion using Jackson's `MAPPER.readTree`.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = escape_java(&json_str);
                let _ = writeln!(out, "        var bodyJson = MAPPER.readTree({response_var}.body());");
                let _ = writeln!(out, "        var expectedJson = MAPPER.readTree(\"{escaped}\");");
                let _ = writeln!(out, "        assertEquals(expectedJson, bodyJson, \"body mismatch\");");
            }
            serde_json::Value::String(s) => {
                let escaped = escape_java(s);
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");"
                );
            }
            other => {
                let escaped = escape_java(&other.to_string());
                let _ = writeln!(
                    out,
                    "        assertEquals(\"{escaped}\", {response_var}.body().trim(), \"body mismatch\");"
                );
            }
        }
    }

    /// Emit partial JSON body assertions: parse once, then assert each expected field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "        var partialJson = MAPPER.readTree({response_var}.body());");
            for (key, val) in obj {
                let escaped_key = escape_java(key);
                let json_str = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = escape_java(&json_str);
                let _ = writeln!(
                    out,
                    "        assertEquals(MAPPER.readTree(\"{escaped_val}\"), partialJson.get(\"{escaped_key}\"), \"body field '{escaped_key}' mismatch\");"
                );
            }
        }
    }

    /// Emit validation-error assertions: parse the body and check each expected message.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[crate::fixture::ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "        var veBody = {response_var}.body();");
        for err in errors {
            let escaped_msg = escape_java(&err.msg);
            let _ = writeln!(
                out,
                "        assertTrue(veBody.contains(\"{escaped_msg}\"), \"expected validation error message: {escaped_msg}\");"
            );
        }
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
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    void test{method_name}() {{");
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"Skipped: Java HttpClient cannot handle 101 Switching Protocols responses\");"
        );
        let _ = writeln!(out, "    }}");
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
    enum_fields: &std::collections::HashMap<String, String>,
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

    // Resolve per-fixture options_type: prefer the java call override, fall back to class-level.
    let effective_options_type: Option<String> = call_overrides
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()));
    let effective_options_type = effective_options_type.as_deref();

    // Resolve per-fixture result_is_simple and result_is_bytes from the call override.
    let effective_result_is_simple =
        call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple || result_is_simple;
    let effective_result_is_bytes = call_overrides.is_some_and(|o| o.result_is_bytes);

    // Check if this test needs ObjectMapper deserialization for json_object args.
    // Strip "input." prefix when looking up field in fixture.input.
    let needs_deser = effective_options_type.is_some()
        && args.iter().any(|arg| {
            if arg.arg_type != "json_object" {
                return false;
            }
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            fixture.input.get(field).is_some_and(|v| !v.is_null() && !v.is_array())
        });

    // Always add throws Exception since the convert method may throw checked exceptions.
    let throws_clause = " throws Exception";

    let _ = writeln!(out, "    @Test");
    let _ = writeln!(out, "    void test{method_name}(){throws_clause} {{");
    let _ = writeln!(out, "        // {description}");

    // Emit builder expressions for json_object args.
    if let (true, Some(opts_type)) = (needs_deser, effective_options_type) {
        for arg in args {
            if arg.arg_type == "json_object" {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                if let Some(val) = fixture.input.get(field) {
                    if !val.is_null() && !val.is_array() {
                        if let Some(obj) = val.as_object() {
                            // Generate builder expression: TypeName.builder().withFieldName(value)...build()
                            let builder_expr = java_builder_expression(
                                obj,
                                opts_type,
                                enum_fields,
                                nested_types,
                                nested_types_optional,
                            );
                            let var_name = &arg.name;
                            let _ = writeln!(out, "        var {var_name} = {builder_expr};");
                        }
                    }
                }
            }
        }
    }

    let (mut setup_lines, args_str) =
        build_args_and_setup(&fixture.input, args, class_name, effective_options_type, &fixture.id);

    // Build visitor if present and add to setup
    let mut visitor_var = String::new();
    let mut has_visitor_fixture = false;
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_var = build_java_visitor(&mut setup_lines, visitor_spec, class_name);
        has_visitor_fixture = true;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    // When visitor is present, attach it to the options parameter
    let final_args = if has_visitor_fixture {
        if args_str.is_empty() {
            // No arguments: just create ConversionOptions with visitor
            format!("new ConversionOptions().withVisitor({})", visitor_var)
        } else if args_str.contains("new ConversionOptions")
            || args_str.contains("ConversionOptionsBuilder")
            || args_str.contains(".builder()")
        {
            // Options are being built (either new ConversionOptions(), builder pattern, or .builder().build())
            // append .withVisitor() call before .build() if present
            if args_str.contains(".build()") {
                // Insert .withVisitor() before the final .build()
                let idx = args_str.rfind(".build()").unwrap();
                format!("{}.withVisitor({}){}", &args_str[..idx], visitor_var, &args_str[idx..])
            } else {
                // Already a chain, just append
                format!("{}.withVisitor({})", args_str, visitor_var)
            }
        } else if args_str.ends_with(", null") {
            // Replace trailing null options with ConversionOptions containing visitor
            let base = &args_str[..args_str.len() - 6];
            format!("{}, new ConversionOptions().withVisitor({})", base, visitor_var)
        } else {
            // args_str is just the html argument(s) — append new ConversionOptions with visitor
            format!("{}, new ConversionOptions().withVisitor({})", args_str, visitor_var)
        }
    } else {
        args_str
    };

    if expects_error {
        let _ = writeln!(
            out,
            "        assertThrows(Exception.class, () -> {class_name}.{function_name}({final_args}));"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    if call_config.returns_void {
        let _ = writeln!(out, "        {class_name}.{function_name}({final_args});");
        let _ = writeln!(out, "    }}");
        return;
    }

    let _ = writeln!(
        out,
        "        var {result_var} = {class_name}.{function_name}({final_args});"
    );

    // Emit a `source` variable for run_query assertions that need the raw bytes.
    let needs_source_var = fixture
        .assertions
        .iter()
        .any(|a| a.assertion_type == "method_result" && a.method.as_deref() == Some("run_query"));
    if needs_source_var {
        // Find the source_code arg to emit a `source` binding.
        if let Some(source_arg) = args.iter().find(|a| a.field == "source_code") {
            let field = source_arg.field.strip_prefix("input.").unwrap_or(&source_arg.field);
            if let Some(val) = fixture.input.get(field) {
                let java_val = json_to_java(val);
                let _ = writeln!(out, "        var source = {java_val}.getBytes();");
            }
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            class_name,
            field_resolver,
            effective_result_is_simple,
            effective_result_is_bytes,
            enum_fields,
        );
    }

    let _ = writeln!(out, "    }}");
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
                "String {} = System.getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\";",
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

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
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
    enum_fields: &std::collections::HashMap<String, String>,
) {
    // Handle synthetic/virtual fields that are computed rather than direct record accessors.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            // ---- ExtractionResult chunk-level computed predicates ----
            "chunks_have_content" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.content() != null && !c.content().isBlank())"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({pred}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({pred}, \"expected false\");");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_heading_context" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.metadata().headingContext().isPresent())"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({pred}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({pred}, \"expected false\");");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().allMatch(c -> c.embedding() != null && !c.embedding().isEmpty())"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({pred}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({pred}, \"expected false\");");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "first_chunk_starts_with_heading" => {
                let pred = format!(
                    "{result_var}.chunks().orElse(java.util.List.of()).stream().findFirst().map(c -> c.metadata().headingContext().isPresent()).orElse(false)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({pred}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({pred}, \"expected false\");");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion on synthetic field '{f}'"
                        );
                    }
                }
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
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let java_val = json_to_java(val);
                            let _ = writeln!(out, "        assertEquals({java_val}, {expr});");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let java_val = json_to_java(val);
                            let _ = writeln!(
                                out,
                                "        assertTrue({expr} > {java_val}, \"expected > {java_val}\");"
                            );
                        }
                    }
                    _ => {
                        let _ = writeln!(out, "        // skipped: unsupported assertion on '{f}'");
                    }
                }
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
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({pred}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({pred}, \"expected false\");");
                    }
                    _ => {
                        let _ = writeln!(out, "        // skipped: unsupported assertion on '{f}'");
                    }
                }
                return;
            }
            // ---- Fields not present on the Java ExtractionResult ----
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "        // skipped: field '{f}' not available on Java ExtractionResult"
                );
                return;
            }
            // ---- metadata not_empty / is_empty: Metadata is a required record, not Optional ----
            // Metadata has no .isEmpty() method; check that at least one optional field is present.
            "metadata" => {
                match assertion.assertion_type.as_str() {
                    "not_empty" => {
                        let _ = writeln!(
                            out,
                            "        assertTrue({result_var}.metadata().title().isPresent() || {result_var}.metadata().subject().isPresent() || !{result_var}.metadata().additional().isEmpty(), \"expected non-empty value\");"
                        );
                        return;
                    }
                    "is_empty" => {
                        let _ = writeln!(
                            out,
                            "        assertFalse({result_var}.metadata().title().isPresent() || {result_var}.metadata().subject().isPresent() || !{result_var}.metadata().additional().isEmpty(), \"expected empty value\");"
                        );
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
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
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
        .is_some_and(|f| enum_fields.contains_key(f) || enum_fields.contains_key(field_resolver.resolve(f)));

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
    let string_expr = if field_is_enum {
        format!("{field_expr}.getValue()")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "        assertEquals({java_val}, {string_expr}.trim());");
                } else if expected.is_number() && field_expr.contains(".orElse(\"\")") {
                    // For numeric "equals" on Optional fields with string fallback,
                    // the field must be Optional<Long/Integer>, not Optional<String>.
                    // Replace the string fallback with a numeric one.
                    let fixed_expr = field_expr.replace(".orElse(\"\")", ".orElse(0L)");
                    let _ = writeln!(out, "        assertEquals({java_val}, {fixed_expr});");
                } else {
                    let _ = writeln!(out, "        assertEquals({java_val}, {field_expr});");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                // For array fields of complex objects (e.g. List<StructureItem>), use .toString()
                // because List.contains(Object) uses equals(), which won't match a String against
                // a record type. Java records produce toString() like "StructureItem[kind=Function, ...]".
                let check_expr = if field_is_array {
                    format!("{string_expr}.toString()")
                } else {
                    string_expr.clone()
                };
                let _ = writeln!(
                    out,
                    "        assertTrue({check_expr}.contains({java_val}), \"expected to contain: \" + {java_val});"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let java_val = json_to_java(val);
                    let check_expr = if field_is_array {
                        format!("{string_expr}.toString()")
                    } else {
                        string_expr.clone()
                    };
                    let _ = writeln!(
                        out,
                        "        assertTrue({check_expr}.contains({java_val}), \"expected to contain: \" + {java_val});"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let check_expr = if field_is_array {
                    format!("{string_expr}.toString()")
                } else {
                    string_expr.clone()
                };
                let _ = writeln!(
                    out,
                    "        assertFalse({check_expr}.contains({java_val}), \"expected NOT to contain: \" + {java_val});"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        assertFalse({field_expr} == null || {field_expr}.isEmpty(), \"expected non-empty value\");"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "        assertTrue({field_expr} == null || {field_expr}.isEmpty(), \"expected empty value\");"
            );
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let java_val = json_to_java(v);
                        format!("{string_expr}.contains({java_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        assertTrue({joined}, \"expected to contain at least one of the specified values\");"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let java_val = json_to_java(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} > {java_val}, \"expected > {java_val}\");"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let java_val = json_to_java(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} < {java_val}, \"expected < {java_val}\");"
                );
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let java_val = json_to_java(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} >= {java_val}, \"expected >= {java_val}\");"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let java_val = json_to_java(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} <= {java_val}, \"expected <= {java_val}\");"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.startsWith({java_val}), \"expected to start with: \" + {java_val});"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.endsWith({java_val}), \"expected to end with: \" + {java_val});"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // byte[] uses `.length` (array field), String uses `.length()` (method).
                    let len_expr = if result_is_bytes {
                        format!("{field_expr}.length")
                    } else {
                        format!("{field_expr}.length()")
                    };
                    let _ = writeln!(
                        out,
                        "        assertTrue({len_expr} >= {n}, \"expected length >= {n}\");"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let len_expr = if result_is_bytes {
                        format!("{field_expr}.length")
                    } else {
                        format!("{field_expr}.length()")
                    };
                    let _ = writeln!(
                        out,
                        "        assertTrue({len_expr} <= {n}, \"expected length <= {n}\");"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({field_expr}.size() >= {n}, \"expected at least {n} elements\");"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertEquals({n}, {field_expr}.size(), \"expected exactly {n} elements\");"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "        assertTrue({field_expr}, \"expected true\");");
        }
        "is_false" => {
            let _ = writeln!(out, "        assertFalse({field_expr}, \"expected false\");");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_java_method_call(result_var, method_name, assertion.args.as_ref(), class_name);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                // Methods that return a collection (List) rather than a scalar.
                let method_returns_collection =
                    matches!(method_name.as_str(), "find_nodes_by_type" | "findNodesByType");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out, "        assertTrue({call_expr});");
                                } else {
                                    let _ = writeln!(out, "        assertFalse({call_expr});");
                                }
                            } else if method_returns_collection {
                                let java_val = json_to_java(val);
                                let _ = writeln!(out, "        assertEquals({java_val}, {call_expr}.size());");
                            } else {
                                let java_val = json_to_java(val);
                                let _ = writeln!(out, "        assertEquals({java_val}, {call_expr});");
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "        assertTrue({call_expr});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        assertFalse({call_expr});");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "        assertTrue({call_expr} >= {n}, \"expected >= {n}\");");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(
                                out,
                                "        assertTrue({call_expr}.size() >= {n}, \"expected at least {n} elements\");"
                            );
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "        assertThrows(Exception.class, () -> {{ {call_expr}; }});");
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let java_val = json_to_java(val);
                            let _ = writeln!(
                                out,
                                "        assertTrue({call_expr}.contains({java_val}), \"expected to contain: \" + {java_val});"
                            );
                        }
                    }
                    other_check => {
                        panic!("Java e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Java e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.matches({java_val}), \"expected value to match regex: \" + {java_val});"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        other => {
            panic!("Java e2e generator: unsupported assertion type: {other}");
        }
    }
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
    enum_fields: &std::collections::HashMap<String, String>,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
) -> String {
    let mut expr = format!("{}.builder()", type_name);
    for (key, val) in obj {
        // Convert snake_case key to camelCase for method name
        let camel_key = key.to_lower_camel_case();
        let method_name = format!("with{}", camel_key.to_upper_camel_case());

        let java_val = match val {
            serde_json::Value::String(s) => {
                // Check if this field is an enum type by looking up in enum_fields.
                // enum_fields is keyed by camelCase names (e.g., "codeBlockStyle"), not snake_case.
                if let Some(enum_type_name) = enum_fields.get(&camel_key) {
                    // Enum field: use the mapped enum type name from the config
                    let variant_name = s.to_upper_camel_case();
                    format!("{}.{}", enum_type_name, variant_name)
                } else if camel_key == "preset" && type_name == "PreprocessingOptions" {
                    // Special case: preset field in PreprocessingOptions maps to PreprocessingPreset
                    let variant_name = s.to_upper_camel_case();
                    format!("PreprocessingPreset.{}", variant_name)
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
                let inner =
                    java_builder_expression(nested, &nested_type, enum_fields, nested_types, nested_types_optional);
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

    setup_lines.push(format!("    @Override public VisitResult {camel_method}({params}) {{"));
    match action {
        CallbackAction::Skip => {
            setup_lines.push("        return VisitResult.skip();".to_string());
        }
        CallbackAction::Continue => {
            setup_lines.push("        return VisitResult.continue_();".to_string());
        }
        CallbackAction::PreserveHtml => {
            setup_lines.push("        return VisitResult.preserveHtml();".to_string());
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_java(output);
            setup_lines.push(format!("        return VisitResult.custom(\"{escaped}\");"));
        }
        CallbackAction::CustomTemplate { template } => {
            // Extract {placeholder} names from the template (in order of appearance).
            // Convert each snake_case placeholder to the camelCase Java variable name,
            // then replace each {placeholder} with %s for String.format.
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
                setup_lines.push(format!("        return VisitResult.custom(\"{escaped}\");"));
            } else {
                let args_str = format_args.join(", ");
                setup_lines.push(format!(
                    "        return VisitResult.custom(String.format(\"{escaped}\", {args_str}));"
                ));
            }
        }
    }
    setup_lines.push("    }".to_string());
}

/// Convert snake_case method names to Java camelCase.
fn method_to_camel(snake: &str) -> String {
    snake.to_lower_camel_case()
}
