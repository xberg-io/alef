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
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
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
            .unwrap_or_else(|| {
                // Derive `<org>/<module>` from the configured repository URL —
                // alef is vendor-neutral, so we don't fall back to a fixed org.
                let org = alef_config
                    .try_github_repo()
                    .ok()
                    .as_deref()
                    .and_then(alef_core::config::derive_repo_org)
                    .unwrap_or_else(|| alef_config.crate_config.name.clone());
                format!("{org}/{}", call.module.replace('_', "-"))
            });
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

        // Derive the e2e composer project metadata from the consumer-binding
        // pkg_name (`<vendor>/<crate>`) and the configured PHP autoload
        // namespace — alef is vendor-neutral, so we don't fall back to a
        // fixed "kreuzberg" string.
        let e2e_vendor = pkg_name.split('/').next().unwrap_or(&pkg_name).to_string();
        let e2e_pkg_name = format!("{e2e_vendor}/e2e-php");
        // PSR-4 autoload keys appear inside a JSON document, so each PHP
        // namespace separator must be JSON-escaped (`\` → `\\`). The trailing
        // pair represents the PHP-mandated trailing `\` (which itself escapes
        // to `\\` in JSON).
        let php_namespace_escaped = alef_config.php_autoload_namespace().replace('\\', "\\\\");
        let e2e_autoload_ns = format!("{php_namespace_escaped}\\\\E2e\\\\");

        // Generate composer.json.
        files.push(GeneratedFile {
            path: output_base.join("composer.json"),
            content: render_composer_json(
                &e2e_pkg_name,
                &e2e_autoload_ns,
                &pkg_name,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
            ),
            generated_header: false,
        });

        // Generate phpunit.xml.
        files.push(GeneratedFile {
            path: output_base.join("phpunit.xml"),
            content: render_phpunit_xml(),
            generated_header: false,
        });

        // Check if any fixture is an HTTP test (needs mock server bootstrap).
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.is_http_test());

        // Generate bootstrap.php that loads both autoloaders and optionally starts the mock server.
        files.push(GeneratedFile {
            path: output_base.join("bootstrap.php"),
            content: render_bootstrap(&pkg_path, has_http_fixtures),
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
    e2e_pkg_name: &str,
    e2e_autoload_ns: &str,
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
    "phpunit/phpunit": "{phpunit}",
    "guzzlehttp/guzzle": "{guzzle}"
  }},"#,
                phpunit = tv::packagist::PHPUNIT,
                guzzle = tv::packagist::GUZZLE,
            )
        }
        crate::config::DependencyMode::Local => format!(
            r#"  "require-dev": {{
    "phpunit/phpunit": "{phpunit}",
    "guzzlehttp/guzzle": "{guzzle}"
  }},"#,
            phpunit = tv::packagist::PHPUNIT,
            guzzle = tv::packagist::GUZZLE,
        ),
    };

    format!(
        r#"{{
  "name": "{e2e_pkg_name}",
  "description": "E2e tests for PHP bindings",
  "type": "project",
{require_section}
  "autoload-dev": {{
    "psr-4": {{
      "{e2e_autoload_ns}": "tests/"
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

fn render_bootstrap(pkg_path: &str, has_http_fixtures: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mock_server_block = if has_http_fixtures {
        r#"
// Spawn the mock HTTP server binary for HTTP fixture tests.
$mockServerBin = __DIR__ . '/../rust/target/release/mock-server';
$fixturesDir = __DIR__ . '/../../fixtures';
if (file_exists($mockServerBin)) {
    $descriptors = [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => STDERR];
    $proc = proc_open([$mockServerBin, $fixturesDir], $descriptors, $pipes);
    if (is_resource($proc)) {
        $line = fgets($pipes[1]);
        if ($line !== false && str_starts_with($line, 'MOCK_SERVER_URL=')) {
            putenv(trim($line));
            $_ENV['MOCK_SERVER_URL'] = trim(substr(trim($line), strlen('MOCK_SERVER_URL=')));
        }
        // Drain stdout in background thread is not possible in PHP; keep pipe open.
        register_shutdown_function(static function () use ($proc, $pipes): void {
            fclose($pipes[0]);
            proc_close($proc);
        });
    }
}
"#
    } else {
        ""
    };
    format!(
        r#"<?php
{header}
declare(strict_types=1);

// Load the e2e project autoloader (PHPUnit, test helpers).
require_once __DIR__ . '/vendor/autoload.php';

// Load the PHP binding package classes via its Composer autoloader.
// The package's autoloader is separate from the e2e project's autoloader
// since the php-ext type prevents direct composer path dependency.
$pkgAutoloader = __DIR__ . '/{pkg_path}/vendor/autoload.php';
if (file_exists($pkgAutoloader)) {{
    require_once $pkgAutoloader;
}}{mock_server_block}
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
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
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
            "        $baseUrl = (string)(getenv('MOCK_SERVER_URL') ?: 'http://localhost:8080');"
        );
        let _ = writeln!(
            out,
            "        $this->httpClient = new Client(['base_uri' => $baseUrl, 'http_errors' => false, 'decode_content' => false, 'allow_redirects' => false]);"
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
    let fixture_id = &fixture.id;

    // HTTP 101 (WebSocket upgrade) causes cURL to treat the connection as an upgrade
    // and fail with "empty reply from server". Skip these tests in the PHP e2e suite
    // since Guzzle cannot assert on WebSocket upgrade responses via regular HTTP.
    let status = http.expected_response.status_code;
    if status == 101 {
        let _ = writeln!(out, "    /** {description} */");
        let _ = writeln!(out, "    public function test_{method_name}(): void");
        let _ = writeln!(out, "    {{");
        let _ = writeln!(
            out,
            "        $this->markTestSkipped('HTTP 101 WebSocket upgrade cannot be tested via Guzzle HTTP client');"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    // Determine body assertion strategy:
    // - String bodies: mock server returns raw text, compare via (string)$response->getBody()
    // - Object/array bodies: use json_decode + assertEquals
    // - Empty string sentinel ("") or null: no body assertion
    let body_is_plain_string =
        matches!(&http.expected_response.body, Some(serde_json::Value::String(s)) if !s.is_empty());
    let has_explicit_body =
        matches!(&http.expected_response.body, Some(v) if !(v.is_null() || v.is_string() && v.as_str() == Some("")));
    // Only call json_decode for non-string bodies (objects, arrays, booleans, numbers).
    let needs_json_body = has_explicit_body && !body_is_plain_string || http.expected_response.body_partial.is_some();

    let _ = writeln!(out, "    /** {description} */");
    let _ = writeln!(out, "    public function test_{method_name}(): void");
    let _ = writeln!(out, "    {{");

    // Build request targeting the mock server's /fixtures/<id> endpoint.
    render_php_http_request(out, &http.request, fixture_id, needs_json_body);

    // Assert status code.
    let _ = writeln!(
        out,
        "        $this->assertEquals({status}, $response->getStatusCode());"
    );

    // For plain string bodies, compare the raw response body string directly.
    if body_is_plain_string {
        if let Some(serde_json::Value::String(expected_str)) = &http.expected_response.body {
            let php_val = format!("\"{}\"", escape_php(expected_str));
            let _ = writeln!(
                out,
                "        $this->assertEquals({php_val}, (string) $response->getBody());"
            );
        }
        // Still assert headers if any.
        render_php_header_assertions(out, &http.expected_response);
        let _ = writeln!(out, "    }}");
        return;
    }

    // Assert response body (JSON decode path).
    render_php_body_assertions(out, &http.expected_response, needs_json_body);

    // Assert response headers.
    render_php_header_assertions(out, &http.expected_response);

    let _ = writeln!(out, "    }}");
}

/// Emit Guzzle request lines inside a PHPUnit test method.
/// `needs_json_body` controls whether a `$body = json_decode(...)` line is emitted.
/// Skip it for responses with no body (204, 304, HEAD, etc.) to avoid JsonException.
fn render_php_http_request(out: &mut String, req: &HttpRequest, fixture_id: &str, needs_json_body: bool) {
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

    // Use the mock server's /fixtures/<id> endpoint.
    let path_lit = format!("\"/fixtures/{}\"", escape_php(fixture_id));
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

    // Decode JSON body for assertions only when body assertions are expected.
    // Omitting json_decode for empty-body responses (204, 304, HEAD, etc.)
    // prevents JsonException on non-JSON or empty response bodies.
    if needs_json_body {
        let _ = writeln!(
            out,
            "        $body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);"
        );
    }
}

/// Emit body assertions for an HTTP expected response.
/// `body_was_decoded` indicates whether `$body` is already in scope from a json_decode call.
fn render_php_body_assertions(out: &mut String, expected: &HttpExpectedResponse, body_was_decoded: bool) {
    if let Some(body) = &expected.body {
        // Skip assertion when body is the empty-string sentinel (means no body expected).
        if !(body.is_string() && body.as_str() == Some("")) {
            let php_val = json_to_php(body);
            let _ = writeln!(out, "        $this->assertEquals({php_val}, $body);");
        }
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
        // Skip validation_error string checks when a full body assertEquals is already
        // generated — it is redundant and json_encode() escapes slashes differently
        // across PHP versions, causing spurious failures.
        if expected.body.is_none() {
            // Ensure $body is available even when it wasn't json_decoded earlier.
            if !body_was_decoded {
                let _ = writeln!(out, "        $body = json_decode((string) $response->getBody(), true);");
            }
            for err in errors {
                let msg_lit = format!("\"{}\"", escape_php(&err.msg));
                let _ = writeln!(
                    out,
                    "        $this->assertStringContainsString({msg_lit}, json_encode($body, JSON_UNESCAPED_SLASHES));"
                );
            }
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
        // The mock server strips content-encoding headers because it serves uncompressed
        // bodies. Skip asserting this header so tests don't fail against the mock server.
        if header_key == "content-encoding" {
            continue;
        }
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

    // Non-HTTP fixture with no assertions: generate a skipped placeholder so
    // PHPUnit does not try to call a method that may not exist on the binding.
    if fixture.assertions.is_empty() {
        let _ = writeln!(
            out,
            "        $this->markTestSkipped('no assertions configured for this fixture in php e2e');"
        );
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
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->content), true)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        $this->assertTrue({pred});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        $this->assertFalse({pred});");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->embedding), true)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        $this->assertTrue({pred});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        $this->assertFalse({pred});");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns array<array<float>> in PHP — no wrapper object.
            // $result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let php_val = json_to_php(val);
                            let _ = writeln!(out, "        $this->assertCount({php_val}, ${result_var});");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let php_val = json_to_php(val);
                            let _ = writeln!(
                                out,
                                "        $this->assertGreaterThanOrEqual({php_val}, count(${result_var}));"
                            );
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "        $this->assertNotEmpty(${result_var});");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "        $this->assertEmpty(${result_var});");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(empty(${result_var}) ? 0 : count(${result_var}[0]))");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let php_val = json_to_php(val);
                            let _ = writeln!(out, "        $this->assertEquals({php_val}, {expr});");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let php_val = json_to_php(val);
                            let _ = writeln!(out, "        $this->assertGreaterThan({php_val}, {expr});");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("array_reduce(${result_var}, fn($carry, $e) => $carry && count($e) > 0, true)")
                    }
                    "embeddings_finite" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && array_reduce($e, fn($c, $v) => $c && is_finite($v), true), true)"
                        )
                    }
                    "embeddings_non_zero" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && count(array_filter($e, fn($v) => $v !== 0.0)) > 0, true)"
                        )
                    }
                    "embeddings_normalized" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && abs(array_sum(array_map(fn($v) => $v * $v, $e)) - 1.0) < 1e-3, true)"
                        )
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "        $this->assertTrue({pred});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        $this->assertFalse({pred});");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "        // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // PHP ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "        // skipped: field '{f}' not available on PHP ExtractionResult"
                );
                return;
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
                let _ = writeln!(
                    out,
                    "        // skipped: result_is_simple, field '{f}' not on simple result type"
                );
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
        "is_false" => {
            let _ = writeln!(out, "        $this->assertFalse({field_expr});");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_php_method_call(result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out, "        $this->assertTrue({call_expr});");
                                } else {
                                    let _ = writeln!(out, "        $this->assertFalse({call_expr});");
                                }
                            } else {
                                let expected = json_to_php(val);
                                let _ = writeln!(out, "        $this->assertEquals({expected}, {call_expr});");
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "        $this->assertTrue({call_expr});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "        $this->assertFalse({call_expr});");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "        $this->assertGreaterThanOrEqual({n}, {call_expr});");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "        $this->assertGreaterThanOrEqual({n}, count({call_expr}));");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "        $this->expectException(\\Exception::class);");
                        let _ = writeln!(out, "        {call_expr};");
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let expected = json_to_php(val);
                            let _ = writeln!(
                                out,
                                "        $this->assertStringContainsString({expected}, {call_expr});"
                            );
                        }
                    }
                    other_check => {
                        panic!("PHP e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("PHP e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let _ = writeln!(
                    out,
                    "        $this->assertMatchesRegularExpression({php_val}, {field_expr});"
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
            panic!("PHP e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build a PHP call expression for a `method_result` assertion on a tree-sitter `Tree`.
///
/// Maps method names to the appropriate PHP static function calls on the
/// `TreeSitterLanguagePack` class (using the ext-php-rs snake_case method names).
fn build_php_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => {
            format!("count(TreeSitterLanguagePack::named_children_info(${result_var}))")
        }
        "root_node_type" => {
            format!("TreeSitterLanguagePack::root_node_info(${result_var})->kind")
        }
        "named_children_count" => {
            format!("count(TreeSitterLanguagePack::named_children_info(${result_var}))")
        }
        "has_error_nodes" => {
            format!("TreeSitterLanguagePack::tree_has_error_nodes(${result_var})")
        }
        "error_count" | "tree_error_count" => {
            format!("TreeSitterLanguagePack::tree_error_count(${result_var})")
        }
        "tree_to_sexp" => {
            format!("TreeSitterLanguagePack::tree_to_sexp(${result_var})")
        }
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("TreeSitterLanguagePack::tree_contains_node_type(${result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("TreeSitterLanguagePack::find_nodes_by_type(${result_var}, \"{node_type}\")")
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
            format!("TreeSitterLanguagePack::run_query(${result_var}, \"{language}\", \"{query_source}\", $source)")
        }
        _ => {
            format!("${result_var}->{method_name}()")
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
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "$ctx, $output",
        "visit_list_start" => "$ctx, $ordered",
        "visit_list_end" => "$ctx, $ordered, $output",
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
            setup_lines.push(format!("        return ['custom' => \"{escaped}\"];"));
        }
        CallbackAction::CustomTemplate { template } => {
            let escaped = escape_php(template);
            setup_lines.push(format!("        return ['custom' => \"{escaped}\"];"));
        }
    }
    setup_lines.push("    }".to_string());
}
