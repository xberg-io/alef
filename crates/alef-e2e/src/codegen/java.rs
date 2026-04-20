//! Java e2e test generator using JUnit 5.
//!
//! Generates `e2e/java/pom.xml` and `src/test/java/dev/kreuzberg/e2e/{Category}Test.java`
//! files from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_java, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Java e2e code generator.
pub struct JavaCodegen;

impl E2eCodegen for JavaCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
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
            .unwrap_or_else(|| alef_config.crate_config.name.to_upper_camel_case());
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
        let result_var = &call.result_var;

        // Resolve package config.
        let java_pkg = e2e_config.resolve_package("java");
        let pkg_name = java_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.crate_config.name.clone());

        // Resolve Java package info for the dependency.
        let java_group_id = alef_config.java_group_id();
        let pkg_version = alef_config.resolved_version().unwrap_or_else(|| "0.1.0".to_string());

        // Generate pom.xml.
        files.push(GeneratedFile {
            path: output_base.join("pom.xml"),
            content: render_pom_xml(&pkg_name, &java_group_id, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate test files per category.
        let test_base = output_base
            .join("src")
            .join("test")
            .join("java")
            .join("dev")
            .join("kreuzberg")
            .join("e2e");

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

            let class_file_name = format!("{}Test.java", sanitize_filename(&group.category).to_upper_camel_case());
            let content = render_test_file(
                &group.category,
                &active,
                &class_name,
                &function_name,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
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

    <groupId>dev.kreuzberg</groupId>
    <artifactId>{artifact_id}</artifactId>
    <version>0.1.0</version>

    <properties>
        <maven.compiler.source>25</maven.compiler.source>
        <maven.compiler.target>25</maven.compiler.target>
        <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
        <junit.version>5.11.4</junit.version>
    </properties>

    <dependencies>
{dep_block}
        <dependency>
            <groupId>com.fasterxml.jackson.core</groupId>
            <artifactId>jackson-databind</artifactId>
            <version>2.18.2</version>
        </dependency>
        <dependency>
            <groupId>com.fasterxml.jackson.datatype</groupId>
            <artifactId>jackson-datatype-jdk8</artifactId>
            <version>2.18.2</version>
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
                <version>3.6.0</version>
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
                <version>3.5.2</version>
                <configuration>
                    <argLine>--enable-preview --enable-native-access=ALL-UNNAMED -Djava.library.path=../../target/release</argLine>
                </configuration>
            </plugin>
        </plugins>
    </build>
</project>
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    class_name: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let test_class_name = format!("{}Test", sanitize_filename(category).to_upper_camel_case());

    // If the class_name is fully qualified (contains '.'), import it and use
    // only the simple name for method calls.  Otherwise use it as-is.
    let (import_path, simple_class) = if class_name.contains('.') {
        let simple = class_name.rsplit('.').next().unwrap_or(class_name);
        (class_name, simple)
    } else {
        ("", class_name)
    };

    let _ = writeln!(out, "package dev.kreuzberg.e2e;");
    let _ = writeln!(out);

    // Check if any fixture uses a json_object arg with options_type (needs ObjectMapper).
    let needs_object_mapper_for_options = options_type.is_some()
        && fixtures.iter().any(|f| {
            args.iter()
                .any(|arg| arg.arg_type == "json_object" && f.input.get(&arg.field).is_some_and(|v| !v.is_null()))
        });
    // Also need ObjectMapper when a handle arg has a non-null config.
    let needs_object_mapper_for_handle = fixtures.iter().any(|f| {
        args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });
    let needs_object_mapper = needs_object_mapper_for_options || needs_object_mapper_for_handle;

    let _ = writeln!(out, "import org.junit.jupiter.api.Test;");
    let _ = writeln!(out, "import static org.junit.jupiter.api.Assertions.*;");
    if !import_path.is_empty() {
        let _ = writeln!(out, "import {import_path};");
    }
    if needs_object_mapper {
        let _ = writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper;");
        let _ = writeln!(out, "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;");
    }
    // Import the options type if tests use it (it's in the same package as the main class).
    if let Some(opts_type) = options_type {
        if needs_object_mapper {
            // Derive the fully-qualified name from the main class import path.
            let opts_package = if !import_path.is_empty() {
                let pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
                format!("{pkg}.{opts_type}")
            } else {
                opts_type.to_string()
            };
            let _ = writeln!(out, "import {opts_package};");
        }
    }
    // Import CrawlConfig when handle args need JSON deserialization.
    if needs_object_mapper_for_handle && !import_path.is_empty() {
        let pkg = import_path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        let _ = writeln!(out, "import {pkg}.CrawlConfig;");
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
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    class_name: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Check if this test needs ObjectMapper deserialization for json_object args.
    let needs_deser = options_type.is_some()
        && args
            .iter()
            .any(|arg| arg.arg_type == "json_object" && fixture.input.get(&arg.field).is_some_and(|v| !v.is_null()));

    // Always add throws Exception since the convert method may throw checked exceptions.
    let throws_clause = " throws Exception";

    let _ = writeln!(out, "    @Test");
    let _ = writeln!(out, "    void test{method_name}(){throws_clause} {{");
    let _ = writeln!(out, "        // {description}");

    // Emit ObjectMapper deserialization bindings for json_object args.
    if let (true, Some(opts_type)) = (needs_deser, options_type) {
        for arg in args {
            if arg.arg_type == "json_object" {
                if let Some(val) = fixture.input.get(&arg.field) {
                    if !val.is_null() {
                        // Fixture keys are camelCase; the Java ConversionOptions record uses
                        // @JsonProperty("snake_case") annotations. Normalize keys so Jackson
                        // can deserialize them correctly.
                        let normalized = super::normalize_json_keys_to_snake_case(val);
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let var_name = &arg.name;
                        let _ = writeln!(
                            out,
                            "        var {var_name} = MAPPER.readValue(\"{}\", {opts_type}.class);",
                            escape_java(&json_str)
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
            "        assertThrows(Exception.class, () -> {class_name}.{function_name}({args_str}));"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    let _ = writeln!(
        out,
        "        var {result_var} = {class_name}.{function_name}({args_str});"
    );

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
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
        return (Vec::new(), json_to_java(input));
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
            let config_value = input.get(&arg.field).unwrap_or(&serde_json::Value::Null);
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

        let val = input.get(&arg.field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: skip entirely.
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0d".to_string(),
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
                parts.push(json_to_java(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
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
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                let accessor = field_resolver.accessor(f, "java", result_var);
                let resolved = field_resolver.resolve(f);
                // Unwrap Optional fields with .orElse("") for string comparisons.
                // Map.get() returns nullable, not Optional, so skip .orElse() for map access.
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    format!("{accessor}.orElse(\"\")")
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
                } else {
                    let _ = writeln!(out, "        assertEquals({java_val}, {field_expr});");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.contains({java_val}), \"expected to contain: \" + {java_val});"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let java_val = json_to_java(val);
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_expr}.contains({java_val}), \"expected to contain: \" + {java_val});"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let java_val = json_to_java(expected);
                let _ = writeln!(
                    out,
                    "        assertFalse({string_expr}.contains({java_val}), \"expected NOT to contain: \" + {java_val});"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        assertFalse({field_expr}.isEmpty(), \"expected non-empty value\");"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "        assertTrue({field_expr}.isEmpty(), \"expected empty value\");"
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
                    let _ = writeln!(
                        out,
                        "        assertTrue({field_expr}.length() >= {n}, \"expected length >= {n}\");"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({field_expr}.length() <= {n}, \"expected length <= {n}\");"
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
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        other => {
            let _ = writeln!(out, "        // TODO: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Java literal string.
fn json_to_java(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_java(s)),
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
            let items: Vec<String> = arr.iter().map(json_to_java).collect();
            format!("java.util.List.of({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_java(&json_str))
        }
    }
}
