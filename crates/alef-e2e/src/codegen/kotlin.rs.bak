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

        // Generate build.gradle.kts.
        files.push(GeneratedFile {
            path: output_base.join("build.gradle.kts"),
            content: render_build_gradle(&pkg_name, &kotlin_pkg_id, &kotlin_version, e2e_config.dep_mode),
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

        // Resolve options_type from override.
        let options_type = overrides.and_then(|o| o.options_type.clone());
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
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
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
// Rendering
// ---------------------------------------------------------------------------

fn render_build_gradle(
    pkg_name: &str,
    kotlin_pkg_id: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!(r#"    testImplementation("{pkg_name}:{pkg_version}")"#)
        }
        crate::config::DependencyMode::Local => {
            // Local mode: reference local JAR or Maven build output.
            format!(r#"    testImplementation(files("../../packages/kotlin/build/libs/{pkg_name}-{pkg_version}.jar"))"#)
        }
    };

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let junit = maven::JUNIT;
    let jackson = maven::JACKSON_E2E;
    let jvm_target = toolchain::JVM_TARGET;
    format!(
        r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {{
    kotlin("jvm") version "{kotlin_plugin}"
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
    environment("java.library.path", "../../target/release")
}}
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
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

    // Check if any fixture uses a json_object arg with options_type (needs ObjectMapper).
    let needs_object_mapper_for_options = options_type.is_some()
        && fixtures.iter().any(|f| {
            args.iter().any(|arg| {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                arg.arg_type == "json_object" && f.input.get(field).is_some_and(|v| !v.is_null())
            })
        });
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
    // Only import the binding class when there are non-HTTP fixtures that call it.
    let has_call_fixtures = fixtures.iter().any(|f| !f.is_http_test());
    if has_call_fixtures && !import_path.is_empty() {
        let _ = writeln!(out, "import {import_path}");
    }
    if needs_object_mapper {
        let _ = writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper");
        let _ = writeln!(out, "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module");
    }
    // Import the options type if tests use it (it's in the same package as the main class).
    if let Some(opts_type) = options_type {
        if needs_object_mapper && has_call_fixtures {
            // Derive the fully-qualified name from the main class import path.
            let opts_package = if !import_path.is_empty() {
                let pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
                format!("{pkg}.{opts_type}")
            } else {
                opts_type.to_string()
            };
            let _ = writeln!(out, "import {opts_package}");
        }
    }
    // Import CrawlConfig when handle args need JSON deserialization.
    if needs_object_mapper_for_handle && !import_path.is_empty() {
        let pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        let _ = writeln!(out, "import {pkg}.CrawlConfig");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "class {test_class_name} {{");

    if needs_object_mapper {
        let _ = writeln!(out);
        let _ = writeln!(out, "    companion object {{");
        let _ = writeln!(
            out,
            "        private val MAPPER = ObjectMapper().registerModule(Jdk8Module())"
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
struct KotlinTestClientRenderer;

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
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "kotlin";
    let call_overrides = call_config.overrides.get(lang);

    // Emit a compilable stub for non-HTTP fixtures that have no Kotlin-specific call
    // override — these fixtures call the default function (e.g., `handleRequest`) which
    // may not exist in the Kotlin binding at this target (e.g., asyncapi, websocket).
    if call_overrides.is_none() {
        let method_name = fixture.id.to_upper_camel_case();
        let description = &fixture.description;
        let _ = writeln!(out, "    @Test");
        let _ = writeln!(out, "    fun test{method_name}() {{");
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"TODO: implement Kotlin e2e test for fixture '{}'\")",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }
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

    // Check if this test needs ObjectMapper deserialization for json_object args.
    let needs_deser = options_type.is_some()
        && args
            .iter()
            .any(|arg| arg.arg_type == "json_object" && fixture.input.get(&arg.field).is_some_and(|v| !v.is_null()));

    let _ = writeln!(out, "    @Test");
    let _ = writeln!(out, "    fun test{method_name}() {{");
    let _ = writeln!(out, "        // {description}");

    // Emit ObjectMapper deserialization bindings for json_object args.
    if let (true, Some(opts_type)) = (needs_deser, options_type) {
        for arg in args {
            if arg.arg_type == "json_object" {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                if let Some(val) = fixture.input.get(field) {
                    if !val.is_null() {
                        let normalized = super::normalize_json_keys_to_snake_case(val);
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let var_name = &arg.name;
                        let _ = writeln!(
                            out,
                            "        val {var_name} = MAPPER.readValue(\"{}\", {opts_type}::class.java)",
                            escape_kotlin(&json_str)
                        );
                    }
                }
            }
        }
    }

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, class_name, options_type, &fixture.id);

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    if expects_error {
        let _ = writeln!(
            out,
            "        assertFailsWith<Exception> {{ {class_name}.{function_name}({args_str}) }}"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    let _ = writeln!(
        out,
        "        val {result_var} = {class_name}.{function_name}({args_str})"
    );

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            class_name,
            field_resolver,
            result_is_simple,
            enum_fields,
        );
    }

    let _ = writeln!(out, "    }}");
}

/// Build setup lines and the argument list for the function call.
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
                "val {} = System.getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
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

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                continue;
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
                // For json_object args with options_type, use the pre-deserialized variable.
                if arg.arg_type == "json_object" && options_type.is_some() {
                    parts.push(arg.name.clone());
                    continue;
                }
                // bytes args must be passed as ByteArray.
                if arg.arg_type == "bytes" {
                    let val = json_to_kotlin(v);
                    parts.push(format!("{val}.toByteArray()"));
                    continue;
                }
                parts.push(json_to_kotlin(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                let accessor = field_resolver.accessor(f, "kotlin", result_var);
                let resolved = field_resolver.resolve(f);
                // In Kotlin, use .orEmpty() for Optional<String> fields.
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    format!("{accessor}.orEmpty()")
                } else {
                    accessor
                }
            }
            _ => result_var.to_string(),
        }
    };

    // For enum fields, use .getValue() to get the string value.
    let string_expr = if field_is_enum {
        format!("{field_expr}.getValue()")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = json_to_kotlin(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {string_expr}.trim())");
                } else {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {field_expr})");
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
            let _ = writeln!(
                out,
                "        assertFalse({field_expr}.isEmpty(), \"expected non-empty value\")"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "        assertTrue({field_expr}.isEmpty(), \"expected empty value\")"
            );
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
                    "        assertTrue({field_expr} > {kotlin_val}, \"expected > {{kotlin_val}}\")"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} < {kotlin_val}, \"expected < {{kotlin_val}}\")"
                );
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} >= {kotlin_val}, \"expected >= {{kotlin_val}}\")"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} <= {kotlin_val}, \"expected <= {{kotlin_val}}\")"
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
                        "        assertTrue({field_expr}.length >= {n}, \"expected length >= {n}\")"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({field_expr}.length <= {n}, \"expected length <= {n}\")"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({field_expr}.size >= {n}, \"expected at least {n} elements\")"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertEquals({n}, {field_expr}.size, \"expected exactly {n} elements\")"
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
                format!("{}d", n)
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
