//! PHP e2e test generator using PHPUnit.
//!
//! Generates `e2e/php/composer.json`, `e2e/php/phpunit.xml`, and
//! `tests/{Category}Test.php` files from JSON fixtures, driven entirely by
//! `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_php, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{
    Assertion, CallbackAction, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpRequest,
};
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

        // Resolve top-level call config to derive class/namespace/factory — these are
        // shared across all categories. Per-fixture call routing (function name, args)
        // is resolved inside render_test_method via e2e_config.resolve_call().
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let extension_name = alef_config.php_extension_name();
        let class_name = overrides
            .and_then(|o| o.class.as_ref())
            .cloned()
            .unwrap_or_else(|| extension_name.to_upper_camel_case());
        let namespace = overrides.and_then(|o| o.module.as_ref()).cloned().unwrap_or_else(|| {
            if extension_name.contains('_') {
                extension_name
                    .split('_')
                    .map(|p| p.to_upper_camel_case())
                    .collect::<Vec<_>>()
                    .join("\\")
            } else {
                extension_name.to_upper_camel_case()
            }
        });
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
        let php_client_factory = overrides.and_then(|o| o.php_client_factory.as_deref());
        let options_via = overrides.and_then(|o| o.options_via.as_deref()).unwrap_or("array");

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
                e2e_config,
                lang,
                &namespace,
                &class_name,
                &test_class,
                &field_resolver,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
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
    "phpunit/phpunit": "^13.1",
    "guzzlehttp/guzzle": "^7.0"
  }},"#
            )
        }
        crate::config::DependencyMode::Local => r#"  "require-dev": {
    "phpunit/phpunit": "^13.1",
    "guzzlehttp/guzzle": "^7.0"
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
         xsi:noNamespaceSchemaLocation="https://schema.phpunit.de/13.1/phpunit.xsd"
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
    e2e_config: &E2eConfig,
    lang: &str,
    namespace: &str,
    class_name: &str,
    test_class: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
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
        let call = e2e_config.resolve_call(f.call.as_deref());
        call.args.iter().filter(|a| a.arg_type == "handle").any(|a| {
            let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
            !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
        })
    });

    // Determine if any fixture is an HTTP test (needs GuzzleHttp).
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    let _ = writeln!(out, "use PHPUnit\\Framework\\TestCase;");
    let _ = writeln!(out, "use {namespace}\\{class_name};");
    if needs_crawl_config_import {
        let _ = writeln!(out, "use {namespace}\\CrawlConfig;");
    }
    if has_http_tests {
        let _ = writeln!(out, "use GuzzleHttp\\Client;");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "/** E2e tests for category: {category}. */");
    let _ = writeln!(out, "final class {test_class} extends TestCase");
    let _ = writeln!(out, "{{");

    // Emit a shared HTTP client property when there are HTTP tests.
    if has_http_tests {
        let _ = writeln!(out, "    private Client $httpClient;");
        let _ = writeln!(out);
        let _ = writeln!(out, "    protected function setUp(): void");
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        parent::setUp();");
        let _ = writeln!(
            out,
            "        $baseUrl = getenv('TEST_SERVER_URL') ?: 'http://localhost:8080';"
        );
        let _ = writeln!(
            out,
            "        $this->httpClient = new Client(['base_uri' => $baseUrl, 'http_errors' => false]);"
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
    }

    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_method(&mut out, fixture, fixture.http.as_ref().unwrap());
        } else {
            render_test_method(
                &mut out,
                fixture,
                e2e_config,
                lang,
                namespace,
                class_name,
                field_resolver,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
            );
        }
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "}}");
    out
}

// ---------------------------------------------------------------------------
// HTTP test rendering
// ---------------------------------------------------------------------------

/// Render a PHPUnit test method for an HTTP server test fixture.
fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    let method_name = sanitize_filename(&fixture.id);
    let description = &fixture.description;

    let _ = writeln!(out, "    /** {description} */");
    let _ = writeln!(out, "    public function test_{method_name}(): void");
    let _ = writeln!(out, "    {{");

    // Build request.
    render_php_http_request(out, &http.request);

    // Assert status code.
    let status = http.expected_response.status_code;
    let _ = writeln!(
        out,
        "        $this->assertEquals({status}, $response->getStatusCode());"
    );

    // Assert response body.
    render_php_body_assertions(out, &http.expected_response);

    // Assert response headers.
    render_php_header_assertions(out, &http.expected_response);

    let _ = writeln!(out, "    }}");
}

/// Emit Guzzle request lines inside a PHPUnit test method.
fn render_php_http_request(out: &mut String, req: &HttpRequest) {
    let method = req.method.to_uppercase();

    // Build options array.
    let mut opts: Vec<String> = Vec::new();

    if let Some(body) = &req.body {
        let php_body = json_to_php(body);
        opts.push(format!("'json' => {php_body}"));
    }

    if !req.headers.is_empty() {
        let header_pairs: Vec<String> = req
            .headers
            .iter()
            .map(|(k, v)| format!("\"{}\" => \"{}\"", escape_php(k), escape_php(v)))
            .collect();
        opts.push(format!("'headers' => [{}]", header_pairs.join(", ")));
    }

    if !req.cookies.is_empty() {
        let cookie_str = req
            .cookies
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        opts.push(format!("'headers' => ['Cookie' => \"{}\"]", escape_php(&cookie_str)));
    }

    if !req.query_params.is_empty() {
        let pairs: Vec<String> = req
            .query_params
            .iter()
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("\"{}\" => \"{}\"", escape_php(k), escape_php(&val_str))
            })
            .collect();
        opts.push(format!("'query' => [{}]", pairs.join(", ")));
    }

    let path_lit = format!("\"{}\"", escape_php(&req.path));
    if opts.is_empty() {
        let _ = writeln!(
            out,
            "        $response = $this->httpClient->request('{method}', {path_lit});"
        );
    } else {
        let _ = writeln!(
            out,
            "        $response = $this->httpClient->request('{method}', {path_lit}, ["
        );
        for opt in &opts {
            let _ = writeln!(out, "            {opt},");
        }
        let _ = writeln!(out, "        ]);");
    }

    // Decode JSON body for assertions.
    let _ = writeln!(
        out,
        "        $body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);"
    );
}

/// Emit body assertions for an HTTP expected response.
fn render_php_body_assertions(out: &mut String, expected: &HttpExpectedResponse) {
    if let Some(body) = &expected.body {
        let php_val = json_to_php(body);
        let _ = writeln!(out, "        $this->assertEquals({php_val}, $body);");
    }
    if let Some(partial) = &expected.body_partial {
        if let Some(obj) = partial.as_object() {
            for (key, val) in obj {
                let php_key = format!("\"{}\"", escape_php(key));
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertEquals({php_val}, $body[{php_key}]);");
            }
        }
    }
    if let Some(errors) = &expected.validation_errors {
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_php(&err.msg));
            let _ = writeln!(
                out,
                "        $this->assertStringContainsString({msg_lit}, json_encode($body));"
            );
        }
    }
}

/// Emit header assertions for an HTTP expected response.
///
/// Special tokens:
/// - `"<<present>>"` — assert the header exists
/// - `"<<absent>>"` — assert the header is absent
/// - `"<<uuid>>"` — assert the header matches a UUID regex
fn render_php_header_assertions(out: &mut String, expected: &HttpExpectedResponse) {
    for (name, value) in &expected.headers {
        let header_key = name.to_lowercase();
        let header_key_lit = format!("\"{}\"", escape_php(&header_key));
        match value.as_str() {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "        $this->assertTrue($response->hasHeader({header_key_lit}));"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "        $this->assertFalse($response->hasHeader({header_key_lit}));"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "        $this->assertMatchesRegularExpression('/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i', $response->getHeaderLine({header_key_lit}));"
                );
            }
            literal => {
                let val_lit = format!("\"{}\"", escape_php(literal));
                let _ = writeln!(
                    out,
                    "        $this->assertEquals({val_lit}, $response->getHeaderLine({header_key_lit}));"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    lang: &str,
    namespace: &str,
    class_name: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
) {
    // Resolve per-fixture call config: supports named calls via fixture.call field.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let call_overrides = call_config.overrides.get(lang);
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // PHP ext-php-rs async methods have an _async suffix.
    if call_config.r#async {
        function_name = format!("{function_name}_async");
    }
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let method_name = sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (mut setup_lines, args_str) =
        build_args_and_setup(&fixture.input, args, class_name, enum_fields, &fixture.id, options_via);

    // Build visitor if present and add to setup
    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_php_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if args_str.is_empty() {
        visitor_arg
    } else {
        format!("{args_str}, {visitor_arg}")
    };

    let call_expr = if php_client_factory.is_some() {
        format!("$client->{function_name}({final_args})")
    } else {
        format!("{class_name}::{function_name}({final_args})")
    };

    let _ = writeln!(out, "    /** {description} */");
    let _ = writeln!(out, "    public function test_{method_name}(): void");
    let _ = writeln!(out, "    {{");

    if let Some(factory) = php_client_factory {
        let _ = writeln!(
            out,
            "        $client = \\{namespace}\\{class_name}::{factory}('test-key');"
        );
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    if expects_error {
        let _ = writeln!(out, "        $this->expectException(\\Exception::class);");
        let _ = writeln!(out, "        {call_expr};");
        let _ = writeln!(out, "    }}");
        return;
    }

    // If no assertion will actually produce a PHPUnit assert call, mark the test
    // as intentionally assertion-free so PHPUnit does not flag it as risky.
    let has_usable = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "error" || a.assertion_type == "not_error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });
    if !has_usable {
        let _ = writeln!(out, "        $this->expectNotToPerformAssertions();");
    }

    let _ = writeln!(out, "        ${result_var} = {call_expr};");

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver, result_is_simple);
    }

    let _ = writeln!(out, "    }}");
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// `options_via` controls how `json_object` args are passed:
/// - `"array"` (default): PHP array literal `["key" => value, ...]`
/// - `"json"`: JSON string via `json_encode([...])` — use when the Rust method accepts `Option<String>`
///
/// Returns `(setup_lines, args_string)`.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    class_name: &str,
    enum_fields: &HashMap<String, String>,
    fixture_id: &str,
    options_via: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        // No args configuration: pass the whole input only if it's non-empty.
        // Functions with no parameters (e.g. list_models) have empty input and get no args.
        let is_empty_input = match input {
            serde_json::Value::Null => true,
            serde_json::Value::Object(m) => m.is_empty(),
            _ => false,
        };
        if is_empty_input {
            return (Vec::new(), String::new());
        }
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
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("${} = {class_name}::{constructor_name}(null);", arg.name,));
            } else {
                let name = &arg.name;
                // Build a CrawlConfig object and set its fields via property assignment.
                // The PHP binding accepts `?CrawlConfig $config` — there is no JSON string
                // variant. Object and array config values are expressed as PHP array literals.
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
                ));
            }
            parts.push(format!("${}", arg.name));
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
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
                    "json_object" if options_via == "json" => "null".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" && !v.is_null() {
                    match options_via {
                        "json" => {
                            // Pass as JSON string via json_encode(); the Rust method accepts Option<String>.
                            parts.push(format!("json_encode({})", json_to_php(v)));
                            continue;
                        }
                        _ => {
                            // Default: PHP array literal with snake_case keys.
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
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        $this->assertCount({n}, {field_expr});");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "        $this->assertTrue({field_expr});");
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

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a PHP visitor object and add setup lines. Returns the visitor expression.
fn build_php_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) -> String {
    setup_lines.push("$visitor = new class {".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_php_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("};".to_string());
    "$visitor".to_string()
}

/// Emit a PHP visitor method for a callback action.
fn emit_php_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    let snake_method = method_name;
    let params = match method_name {
        "visit_link" => "$ctx, $href, $text, $title",
        "visit_image" => "$ctx, $src, $alt, $title",
        "visit_heading" => "$ctx, $level, $text, $id",
        "visit_code_block" => "$ctx, $lang, $code",
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
        | "visit_definition_description" => "$ctx, $text",
        "visit_text" => "$ctx, $text",
        "visit_list_item" => "$ctx, $ordered, $marker, $text",
        "visit_blockquote" => "$ctx, $content, $depth",
        "visit_table_row" => "$ctx, $cells, $isHeader",
        "visit_custom_element" => "$ctx, $tagName, $html",
        "visit_form" => "$ctx, $actionUrl, $method",
        "visit_input" => "$ctx, $inputType, $name, $value",
        "visit_audio" | "visit_video" | "visit_iframe" => "$ctx, $src",
        "visit_details" => "$ctx, $isOpen",
        _ => "$ctx",
    };

    setup_lines.push(format!("    public function {snake_method}({params}) {{"));
    match action {
        CallbackAction::Skip => {
            setup_lines.push("        return 'skip';".to_string());
        }
        CallbackAction::Continue => {
            setup_lines.push("        return 'continue';".to_string());
        }
        CallbackAction::PreserveHtml => {
            setup_lines.push("        return 'preserve_html';".to_string());
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_php(output);
            setup_lines.push(format!("        return ['custom' => {escaped}];"));
        }
        CallbackAction::CustomTemplate { template } => {
            setup_lines.push(format!("        return ['custom' => \"{template}\"];"));
        }
    }
    setup_lines.push("    }".to_string());
}
