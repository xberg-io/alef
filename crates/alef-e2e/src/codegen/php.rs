//! PHP e2e test generator using PHPUnit.
//!
//! Generates `e2e/php/composer.json`, `e2e/php/phpunit.xml`, and
//! `tests/{Category}Test.php` files from JSON fixtures, driven entirely by
//! `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_php, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// PHP e2e code generator.
pub struct PhpCodegen;

impl E2eCodegen for PhpCodegen {
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
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let class_name = overrides
            .and_then(|o| o.class.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.crate_config.name.to_upper_camel_case());
        let namespace = overrides.and_then(|o| o.module.as_ref()).cloned().unwrap_or_else(|| {
            if call.module.is_empty() {
                "Kreuzberg".to_string()
            } else {
                call.module.to_upper_camel_case()
            }
        });
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
        let result_var = &call.result_var;

        // Resolve package config.
        let php_pkg = e2e_config.resolve_package("php");
        let pkg_name = php_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("kreuzberg/{}", call.module.replace('_', "-")));
        let pkg_path = php_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/php".to_string());
        let pkg_version = php_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate composer.json.
        files.push(GeneratedFile {
            path: output_base.join("composer.json"),
            content: render_composer_json(&pkg_name, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate phpunit.xml.
        files.push(GeneratedFile {
            path: output_base.join("phpunit.xml"),
            content: render_phpunit_xml(),
            generated_header: false,
        });

        // Generate bootstrap.php that loads both autoloaders.
        files.push(GeneratedFile {
            path: output_base.join("bootstrap.php"),
            content: render_bootstrap(&pkg_path),
            generated_header: true,
        });

        // Generate test files per category.
        let tests_base = output_base.join("tests");
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

            let test_class = format!("{}Test", sanitize_filename(&group.category).to_upper_camel_case());
            let filename = format!("{test_class}.php");
            let content = render_test_file(
                &group.category,
                &active,
                &namespace,
                &class_name,
                &function_name,
                result_var,
                &test_class,
                &e2e_config.call.args,
                &field_resolver,
                enum_fields,
                result_is_simple,
            );
            files.push(GeneratedFile {
                path: tests_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "php"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_composer_json(
    pkg_name: &str,
    _pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let require_section = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!(
                r#"  "require": {{
    "{pkg_name}": "{pkg_version}"
  }},
  "require-dev": {{
    "phpunit/phpunit": "^11.0"
  }},"#
            )
        }
        crate::config::DependencyMode::Local => r#"  "require-dev": {
    "phpunit/phpunit": "^11.0"
  },"#
        .to_string(),
    };

    format!(
        r#"{{
  "name": "kreuzberg/e2e-php",
  "description": "E2e tests for PHP bindings",
  "type": "project",
{require_section}
  "autoload-dev": {{
    "psr-4": {{
      "Kreuzberg\\E2e\\": "tests/"
    }}
  }}
}}
"#
    )
}

fn render_phpunit_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<phpunit xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:noNamespaceSchemaLocation="https://schema.phpunit.de/11.0/phpunit.xsd"
         bootstrap="bootstrap.php"
         colors="true"
         failOnRisky="true"
         failOnWarning="true">
    <testsuites>
        <testsuite name="e2e">
            <directory>tests</directory>
        </testsuite>
    </testsuites>
</phpunit>
"#
    .to_string()
}

fn render_bootstrap(pkg_path: &str) -> String {
    format!(
        r#"<?php
// This file is auto-generated by alef. DO NOT EDIT.

declare(strict_types=1);

// Load the e2e project autoloader (PHPUnit, test helpers).
require_once __DIR__ . '/vendor/autoload.php';

// Load the PHP binding package classes via its Composer autoloader.
// The package's autoloader is separate from the e2e project's autoloader
// since the php-ext type prevents direct composer path dependency.
$pkgAutoloader = __DIR__ . '/{pkg_path}/vendor/autoload.php';
if (file_exists($pkgAutoloader)) {{
    require_once $pkgAutoloader;
}}
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    namespace: &str,
    class_name: &str,
    function_name: &str,
    result_var: &str,
    test_class: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "<?php");
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out);
    let _ = writeln!(out, "declare(strict_types=1);");
    let _ = writeln!(out);
    let _ = writeln!(out, "namespace Kreuzberg\\E2e;");
    let _ = writeln!(out);
    // Determine if any handle arg has a non-null config (needs CrawlConfig import).
    let needs_crawl_config_import = fixtures.iter().any(|f| {
        args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });

    let _ = writeln!(out, "use PHPUnit\\Framework\\TestCase;");
    let _ = writeln!(out, "use {namespace}\\{class_name};");
    if needs_crawl_config_import {
        let _ = writeln!(out, "use {namespace}\\CrawlConfig;");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "final class {test_class} extends TestCase");
    let _ = writeln!(out, "{{");

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_method(
            &mut out,
            fixture,
            class_name,
            function_name,
            result_var,
            args,
            field_resolver,
            enum_fields,
            result_is_simple,
        );
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
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
    field_resolver: &FieldResolver,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
) {
    let method_name = sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, class_name, enum_fields, &fixture.id);

    let call_expr = format!("{class_name}::{function_name}({args_str})");

    let _ = writeln!(out, "    /** {description} */");
    let _ = writeln!(out, "    public function test_{method_name}(): void");
    let _ = writeln!(out, "    {{");

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    if expects_error {
        let _ = writeln!(out, "        $this->expectException(\\Exception::class);");
        let _ = writeln!(out, "        {call_expr};");
        let _ = writeln!(out, "    }}");
        return;
    }

    let _ = writeln!(out, "        ${result_var} = {call_expr};");

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver, result_is_simple);
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
    enum_fields: &HashMap<String, String>,
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), json_to_php(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "${} = getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';",
                arg.name,
            ));
            parts.push(format!("${}", arg.name));
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a createEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let config_value = input.get(&arg.field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("${} = {class_name}::{constructor_name}(null);", arg.name,));
            } else {
                let name = &arg.name;
                // Check if config has complex fields (objects/maps) that can't be
                // set via PHP property assignment. If so, use createEngineFromJson
                // which deserializes via core serde (handles auth, browser, proxy etc.).
                let has_complex = config_value
                    .as_object()
                    .is_some_and(|obj| obj.values().any(|v| v.is_object() || v.is_array()));
                if has_complex {
                    let json_str = serde_json::to_string(config_value).unwrap_or_default();
                    let escaped = json_str.replace('\'', "\\'");
                    setup_lines.push(format!(
                        "${} = {class_name}::createEngineFromJson('{escaped}');",
                        arg.name,
                    ));
                } else {
                    setup_lines.push(format!("${name}_config = CrawlConfig::default();"));
                    if let Some(obj) = config_value.as_object() {
                        for (key, val) in obj {
                            let php_val = json_to_php(val);
                            setup_lines.push(format!("${name}_config->{key} = {php_val};"));
                        }
                    }
                    setup_lines.push(format!(
                        "${} = {class_name}::{constructor_name}(${name}_config);",
                        arg.name,
                        name = name,
                    ));
                }
            }
            parts.push(format!("${}", arg.name));
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
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For json_object args, convert keys to snake_case and enum values appropriately.
                if arg.arg_type == "json_object" && !v.is_null() {
                    if let Some(obj) = v.as_object() {
                        let items: Vec<String> = obj
                            .iter()
                            .map(|(k, vv)| {
                                let snake_key = k.to_snake_case();
                                let php_val = if enum_fields.contains_key(k) {
                                    if let Some(s) = vv.as_str() {
                                        let snake_val = s.to_snake_case();
                                        format!("\"{}\"", escape_php(&snake_val))
                                    } else {
                                        json_to_php(vv)
                                    }
                                } else {
                                    json_to_php(vv)
                                };
                                format!("\"{}\" => {}", escape_php(&snake_key), php_val)
                            })
                            .collect();
                        parts.push(format!("[{}]", items.join(", ")));
                        continue;
                    }
                }
                parts.push(json_to_php(v));
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
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields
    // (e.g., metadata, document, structure) since the binding returns a plain value.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                let _ = writeln!(out, "        // TODO: skipped (result_is_simple, field: {f})");
                return;
            }
        }
    }

    let field_expr = if result_is_simple {
        format!("${result_var}")
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "php", &format!("${result_var}")),
            _ => format!("${result_var}"),
        }
    };

    // For string equality, trim trailing whitespace to handle trailing newlines.
    let trimmed_field_expr = if result_is_simple {
        format!("trim(${result_var})")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(out, "        $this->assertEquals({php_val}, {trimmed_field_expr});");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(
                    out,
                    "        $this->assertStringContainsString({php_val}, {field_expr});"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let php_val = json_to_php(val);
                    let _ = writeln!(
                        out,
                        "        $this->assertStringContainsString({php_val}, {field_expr});"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(
                    out,
                    "        $this->assertStringNotContainsString({php_val}, {field_expr});"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "        $this->assertNotEmpty({field_expr});");
        }
        "is_empty" => {
            let _ = writeln!(out, "        $this->assertEmpty({trimmed_field_expr});");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let _ = writeln!(out, "        $found = false;");
                for val in values {
                    let php_val = json_to_php(val);
                    let _ = writeln!(
                        out,
                        "        if (str_contains({field_expr}, {php_val})) {{ $found = true; }}"
                    );
                }
                let _ = writeln!(
                    out,
                    "        $this->assertTrue($found, 'expected to contain at least one of the specified values');"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertGreaterThan({php_val}, {field_expr});");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertLessThan({php_val}, {field_expr});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertGreaterThanOrEqual({php_val}, {field_expr});");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertLessThanOrEqual({php_val}, {field_expr});");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(out, "        $this->assertStringStartsWith({php_val}, {field_expr});");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(out, "        $this->assertStringEndsWith({php_val}, {field_expr});");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        $this->assertGreaterThanOrEqual({n}, strlen({field_expr}));"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        $this->assertLessThanOrEqual({n}, strlen({field_expr}));");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        $this->assertGreaterThanOrEqual({n}, count({field_expr}));"
                    );
                }
            }
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

/// Convert a `serde_json::Value` to a PHP literal string.
fn json_to_php(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_php(s)),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_php).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\" => {}", escape_php(k), json_to_php(v)))
                .collect();
            format!("[{}]", items.join(", "))
        }
    }
}
