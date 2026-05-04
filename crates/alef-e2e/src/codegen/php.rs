//! PHP e2e test generator using PHPUnit.
//!
//! Generates `e2e/php/composer.json`, `e2e/php/phpunit.xml`, and
//! `tests/{Category}Test.php` files from JSON fixtures, driven entirely by
//! `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_php, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture, ValidationErrorExpectation};
use alef_backend_php::naming::php_autoload_namespace;
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// PHP e2e code generator.
pub struct PhpCodegen;

impl E2eCodegen for PhpCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve top-level call config to derive class/namespace/factory — these are
        // shared across all categories. Per-fixture call routing (function name, args)
        // is resolved inside render_test_method via e2e_config.resolve_call().
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let extension_name = config.php_extension_name();
        let class_name = overrides
            .and_then(|o| o.class.as_ref())
            .cloned()
            .map(|cn| cn.split('\\').next_back().unwrap_or(&cn).to_string())
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
                let org = config
                    .try_github_repo()
                    .ok()
                    .as_deref()
                    .and_then(alef_core::config::derive_repo_org)
                    .unwrap_or_else(|| config.name.clone());
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
        let php_namespace_escaped = php_autoload_namespace(config).replace('\\', "\\\\");
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

        // Generate run_tests.php that loads the extension and invokes phpunit.
        files.push(GeneratedFile {
            path: output_base.join("run_tests.php"),
            content: render_run_tests_php(&extension_name),
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
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let (require_section, autoload_section) = match dep_mode {
        crate::config::DependencyMode::Registry => {
            let require = format!(
                r#"  "require": {{
    "{pkg_name}": "{pkg_version}"
  }},
  "require-dev": {{
    "phpunit/phpunit": "{phpunit}",
    "guzzlehttp/guzzle": "{guzzle}"
  }},"#,
                phpunit = tv::packagist::PHPUNIT,
                guzzle = tv::packagist::GUZZLE,
            );
            (require, String::new())
        }
        crate::config::DependencyMode::Local => {
            let require = format!(
                r#"  "require-dev": {{
    "phpunit/phpunit": "{phpunit}",
    "guzzlehttp/guzzle": "{guzzle}"
  }},"#,
                phpunit = tv::packagist::PHPUNIT,
                guzzle = tv::packagist::GUZZLE,
            );
            // For local mode, add autoload for the local package source.
            // Extract the namespace from pkg_name (org/module) and map it to src/.
            let pkg_namespace = pkg_name
                .split('/')
                .nth(1)
                .unwrap_or(pkg_name)
                .split('-')
                .map(heck::ToUpperCamelCase::to_upper_camel_case)
                .collect::<Vec<_>>()
                .join("\\");
            let autoload = format!(
                r#"
  "autoload": {{
    "psr-4": {{
      "{}\\": "{}/src/"
    }}
  }},"#,
                pkg_namespace.replace('\\', "\\\\"),
                pkg_path
            );
            (require, autoload)
        }
    };

    format!(
        r#"{{
  "name": "{e2e_pkg_name}",
  "description": "E2e tests for PHP bindings",
  "type": "project",
{require_section}{autoload_section}
  "autoload-dev": {{
    "psr-4": {{
      "{e2e_autoload_ns}": "tests/"
    }}
  }},
  "scripts": {{
    "test": "php run_tests.php"
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

fn render_run_tests_php(extension_name: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let ext_lib_name = format!("lib{extension_name}_php");
    let ext_class_name = format!("{}_php", extension_name);
    format!(
        r#"#!/usr/bin/env php
<?php
{header}
declare(strict_types=1);

// Determine platform-specific extension suffix.
$extSuffix = match (PHP_OS_FAMILY) {{
    'Darwin' => '.dylib',
    default => '.so',
}};
$extPath = __DIR__ . '/../../target/release/{ext_lib_name}' . $extSuffix;

// If extension is not already loaded and the extension file exists, we need to
// restart PHP with the extension enabled via command-line.
if (!extension_loaded('{ext_class_name}') && file_exists($extPath)) {{
    // Reconstruct the command with the extension flag.
    $php = PHP_BINARY;
    $extFlag = "-d";
    $extVal = "extension=" . $extPath;
    $phpunitPath = __DIR__ . '/vendor/bin/phpunit';

    // Build the full command: php -d extension=... vendor/bin/phpunit [args...]
    $cmd = array_merge(
        [$php, $extFlag, $extVal],
        [$phpunitPath],
        array_slice($GLOBALS['argv'], 1)
    );

    // Execute and exit with the same code.
    passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);
    exit($exitCode);
}}

// Extension is already loaded (either built-in or via this script after restart).
// Invoke PHPUnit normally.
$phpunitPath = __DIR__ . '/vendor/bin/phpunit';
if (!file_exists($phpunitPath)) {{
    echo "PHPUnit not found at $phpunitPath. Run 'composer install' first.\n";
    exit(1);
}}

require $phpunitPath;
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
    let _ = writeln!(out, "namespace {namespace}\\E2e;");
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

    // Collect options_type class names that need `use` imports (one import per unique name).
    let mut options_type_imports: Vec<String> = fixtures
        .iter()
        .flat_map(|f| {
            let call = e2e_config.resolve_call(f.call.as_deref());
            let php_override = call.overrides.get(lang);
            let opt_type = php_override.and_then(|o| o.options_type.as_deref()).or_else(|| {
                e2e_config
                    .call
                    .overrides
                    .get(lang)
                    .and_then(|o| o.options_type.as_deref())
            });
            opt_type.map(|t| t.to_string()).into_iter()
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    options_type_imports.sort();

    let _ = writeln!(out, "use PHPUnit\\Framework\\TestCase;");
    let _ = writeln!(out, "use {namespace}\\{class_name};");
    if needs_crawl_config_import {
        let _ = writeln!(out, "use {namespace}\\CrawlConfig;");
    }
    for type_name in &options_type_imports {
        if type_name != class_name {
            let _ = writeln!(out, "use {namespace}\\{type_name};");
        }
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
// HTTP test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Thin renderer that emits PHPUnit test methods targeting a mock server via
/// Guzzle. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct PhpTestClientRenderer;

impl client::TestClientRenderer for PhpTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "php"
    }

    /// Convert a fixture id to a PHP-valid identifier (snake_case via `sanitize_filename`).
    fn sanitize_test_name(&self, id: &str) -> String {
        sanitize_filename(id)
    }

    /// Emit `/** {description} */ public function test_{fn_name}(): void {`.
    ///
    /// When `skip_reason` is `Some`, emits a `markTestSkipped(...)` body and the
    /// shared driver calls `render_test_close` immediately after, so the closing
    /// brace is emitted symmetrically.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "    /** {description} */");
        let _ = writeln!(out, "    public function test_{fn_name}(): void");
        let _ = writeln!(out, "    {{");
        if let Some(reason) = skip_reason {
            let reason_lit = format!("\"{}\"", escape_php(reason));
            let _ = writeln!(out, "        $this->markTestSkipped({reason_lit});");
        }
    }

    /// Emit the closing `}` for a test method.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
    }

    /// Emit a Guzzle request to the mock server's `/fixtures/<fixture_id>` endpoint.
    ///
    /// The fixture id is extracted from the path (which the mock server routes as
    /// `/fixtures/<id>`). `$response` is bound for subsequent assertion methods.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();

        // Build Guzzle options array.
        let mut opts: Vec<String> = Vec::new();

        if let Some(body) = ctx.body {
            let php_body = json_to_php(body);
            opts.push(format!("'json' => {php_body}"));
        }

        // Merge explicit headers and content_type hint.
        let mut header_pairs: Vec<String> = Vec::new();
        if let Some(ct) = ctx.content_type {
            // Only emit if not already in ctx.headers (avoid duplicate Content-Type).
            if !ctx.headers.keys().any(|k| k.to_lowercase() == "content-type") {
                header_pairs.push(format!("\"Content-Type\" => \"{}\"", escape_php(ct)));
            }
        }
        for (k, v) in ctx.headers {
            header_pairs.push(format!("\"{}\" => \"{}\"", escape_php(k), escape_php(v)));
        }
        if !header_pairs.is_empty() {
            opts.push(format!("'headers' => [{}]", header_pairs.join(", ")));
        }

        if !ctx.cookies.is_empty() {
            let cookie_str = ctx
                .cookies
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("; ");
            opts.push(format!("'headers' => ['Cookie' => \"{}\"]", escape_php(&cookie_str)));
        }

        if !ctx.query_params.is_empty() {
            let pairs: Vec<String> = ctx
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

        let path_lit = format!("\"{}\"", escape_php(ctx.path));
        if opts.is_empty() {
            let _ = writeln!(
                out,
                "        ${} = $this->httpClient->request('{method}', {path_lit});",
                ctx.response_var,
            );
        } else {
            let _ = writeln!(
                out,
                "        ${} = $this->httpClient->request('{method}', {path_lit}, [",
                ctx.response_var,
            );
            for opt in &opts {
                let _ = writeln!(out, "            {opt},");
            }
            let _ = writeln!(out, "        ]);");
        }
    }

    /// Emit `$this->assertEquals(status, $response->getStatusCode())`.
    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "        $this->assertEquals({status}, $response->getStatusCode());"
        );
    }

    /// Emit a header assertion using `$response->getHeaderLine(...)` or
    /// `$response->hasHeader(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_key_lit = format!("\"{}\"", escape_php(&header_key));
        match expected {
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

    /// Emit a JSON body equality assertion.
    ///
    /// Plain string bodies are compared against `(string) $response->getBody()` directly;
    /// structured bodies (objects, arrays, booleans, numbers) are decoded via `json_decode`
    /// and compared with `assertEquals`.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::String(s) if !s.is_empty() => {
                let php_val = format!("\"{}\"", escape_php(s));
                let _ = writeln!(
                    out,
                    "        $this->assertEquals({php_val}, (string) $response->getBody());"
                );
            }
            _ => {
                let php_val = json_to_php(expected);
                let _ = writeln!(
                    out,
                    "        $body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);"
                );
                let _ = writeln!(out, "        $this->assertEquals({php_val}, $body);");
            }
        }
    }

    /// Emit partial body assertions: one `assertEquals` per field in `expected`.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(
                out,
                "        $body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);"
            );
            for (key, val) in obj {
                let php_key = format!("\"{}\"", escape_php(key));
                let php_val = json_to_php(val);
                let _ = writeln!(out, "        $this->assertEquals({php_val}, $body[{php_key}]);");
            }
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` against the
    /// JSON-encoded body string (PHP binding returns ProblemDetails with `errors` array).
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "        $body = json_decode((string) $response->getBody(), true);");
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_php(&err.msg));
            let _ = writeln!(
                out,
                "        $this->assertStringContainsString({msg_lit}, json_encode($body, JSON_UNESCAPED_SLASHES));"
            );
        }
    }
}

/// Render a PHPUnit test method for an HTTP server test fixture via the shared driver.
///
/// Handles the one PHP-specific pre-condition: HTTP 101 (WebSocket upgrade) causes
/// cURL/Guzzle to fail; it is emitted as a `markTestSkipped` stub directly.
fn render_http_test_method(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    // HTTP 101 (WebSocket upgrade) causes cURL to treat the connection as an upgrade
    // and fail with "empty reply from server". Skip these tests in the PHP e2e suite
    // since Guzzle cannot assert on WebSocket upgrade responses via regular HTTP.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_filename(&fixture.id);
        let description = &fixture.description;
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

    client::http_call::render_http_test(out, &PhpTestClientRenderer, fixture);
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
    let has_override = call_overrides.is_some_and(|o| o.function.is_some());
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // PHP ext-php-rs async methods have an _async suffix, but only if the function
    // name was not explicitly overridden. When a language-specific override provides
    // a function name, use it as-is without modification.
    if !has_override && call_config.r#async {
        function_name = format!("{function_name}_async");
    }
    // PHP wrapper classes use lowerCamelCase method names (e.g. getLanguage, downloadAll).
    // Convert the Rust snake_case name only when no explicit override is provided.
    if !has_override {
        function_name = function_name.to_lower_camel_case();
    }
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let method_name = sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve options_type for this call's PHP override, with fallback to the top-level call override.
    let call_options_type = call_overrides.and_then(|o| o.options_type.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.options_type.as_deref())
    });

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        class_name,
        enum_fields,
        &fixture.id,
        options_via,
        call_options_type,
    );

    // Build visitor if present and add to setup
    let mut options_already_created = !args_str.is_empty() && args_str == "$options";
    if let Some(visitor_spec) = &fixture.visitor {
        build_php_visitor(&mut setup_lines, visitor_spec);
        if !options_already_created {
            // Create options via builder with visitor.
            // Note: PHP ext-php-rs bridge limitations mean the visitor() method ignores
            // its parameter and passes None to the inner builder. This is a known limitation
            // in the PHP backend that needs a proper visitor bridge implementation.
            setup_lines.push("$builder = \\HtmlToMarkdown\\ConversionOptions::builder();".to_string());
            setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
            options_already_created = true;
        }
    }

    let final_args = if options_already_created {
        if args_str.is_empty() || args_str == "$options" {
            "$options".to_string()
        } else {
            format!("{args_str}, $options")
        }
    } else {
        args_str
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
/// `options_type` is the PHP class name (e.g. `"ProcessConfig"`) used when constructing options
/// via `ClassName::from_json(json_encode([...]))`. Required when `options_via` is not `"json"` and
/// the binding accepts a typed config object.
///
/// Returns `(setup_lines, args_string)`.
/// Emit PHP batch item array constructors for BatchBytesItem or BatchFileItem arrays.
fn emit_php_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
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
                                    .filter_map(|v| v.as_u64())
                                    .map(|n| format!("\\x{:02x}", n))
                                    .collect();
                                format!("\"{}\"", bytes.join(""))
                            } else {
                                "\"\"".to_string()
                            };
                            Some(format!(
                                "new {}(content: {}, mimeType: \"{}\")",
                                elem_type, content_code, mime_type
                            ))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            Some(format!("new {}(path: \"{}\")", elem_type, path))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    class_name: &str,
    _enum_fields: &HashMap<String, String>,
    fixture_id: &str,
    options_via: &str,
    options_type: Option<&str>,
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
                // Use CrawlConfig::from_json() instead of direct property assignment.
                // ext-php-rs doesn't support writable #[php(prop)] fields for complex types,
                // so serialize the config to JSON and use from_json() to construct it.
                setup_lines.push(format!(
                    "${name}_config = CrawlConfig::from_json(json_encode({}));",
                    json_to_php(config_value)
                ));
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
                    // Check for batch item arrays first
                    if let Some(elem_type) = &arg.element_type {
                        if (elem_type == "BatchBytesItem" || elem_type == "BatchFileItem") && v.is_array() {
                            parts.push(emit_php_batch_item_array(v, elem_type));
                            continue;
                        }
                    }
                    match options_via {
                        "json" => {
                            // Pass as JSON string via json_encode(); the Rust method accepts Option<String>.
                            parts.push(format!("json_encode({})", json_to_php(v)));
                            continue;
                        }
                        _ => {
                            if let Some(type_name) = options_type {
                                // Use TypeName::from_json(json_encode([...])) to construct the
                                // typed config object. ext-php-rs structs expose a from_json()
                                // static method that accepts a JSON string.
                                let arg_var = format!("${}", arg.name);
                                setup_lines.push(format!(
                                    "{arg_var} = {type_name}::from_json(json_encode({}));",
                                    json_to_php(v)
                                ));
                                parts.push(arg_var);
                                continue;
                            }
                            // Fallback: builder pattern when no options_type is configured.
                            // This path is kept for backwards compatibility with projects
                            // that use a builder-style API without from_json().
                            if let Some(obj) = v.as_object() {
                                setup_lines.push("$builder = $this->createDefaultOptionsBuilder();".to_string());
                                for (k, vv) in obj {
                                    let snake_key = k.to_snake_case();
                                    if snake_key == "preprocessing" {
                                        if let Some(prep_obj) = vv.as_object() {
                                            let enabled =
                                                prep_obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                                            let preset =
                                                prep_obj.get("preset").and_then(|v| v.as_str()).unwrap_or("Minimal");
                                            let remove_navigation = prep_obj
                                                .get("remove_navigation")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(true);
                                            let remove_forms =
                                                prep_obj.get("remove_forms").and_then(|v| v.as_bool()).unwrap_or(true);
                                            setup_lines.push(format!(
                                                "$preprocessing = $this->createPreprocessingOptions({}, {}, {}, {});",
                                                if enabled { "true" } else { "false" },
                                                json_to_php(&serde_json::Value::String(preset.to_string())),
                                                if remove_navigation { "true" } else { "false" },
                                                if remove_forms { "true" } else { "false" }
                                            ));
                                            setup_lines.push(
                                                "$builder = $builder->preprocessing($preprocessing);".to_string(),
                                            );
                                        }
                                    }
                                }
                                setup_lines.push("$options = $builder->build();".to_string());
                                parts.push("$options".to_string());
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

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "php", &format!("${result_var}")),
        _ if result_is_simple => {
            // When result_is_simple, default to accessing the 'content' field
            field_resolver.accessor("content", "php", &format!("${result_var}"))
        }
        _ => format!("${result_var}"),
    };

    // For string equality, trim trailing whitespace to handle trailing newlines.
    // Only apply trim() when the expected value is a string — calling trim() on int/bool
    // throws TypeError in PHP 8.4+.
    let trimmed_field_expr_for = |expected: &serde_json::Value| -> String {
        if expected.is_string() {
            format!("trim({})", field_expr)
        } else {
            field_expr.clone()
        }
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let php_val = json_to_php(expected);
                let effective_expr = trimmed_field_expr_for(expected);
                let _ = writeln!(out, "        $this->assertEquals({php_val}, {effective_expr});");
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
            let _ = writeln!(out, "        $this->assertEmpty({field_expr});");
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

/// Build a PHP visitor object and add setup lines. The visitor is assigned to $visitor variable.
fn build_php_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) {
    setup_lines.push("$visitor = new class {".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_php_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("};".to_string());
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
