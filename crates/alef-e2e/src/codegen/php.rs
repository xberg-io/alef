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
            .or_else(|| config.resolved_version())
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

        // Check if any fixture needs a mock HTTP server (either http-shape or
        // liter-llm mock_response-shape) so bootstrap.php spawns it.
        let has_http_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Check if any fixture uses file_path or bytes args (needs chdir to test_documents).
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Generate bootstrap.php that loads both autoloaders and optionally starts the mock server.
        files.push(GeneratedFile {
            path: output_base.join("bootstrap.php"),
            content: render_bootstrap(&pkg_path, has_http_fixtures, has_file_fixtures),
            generated_header: true,
        });

        // Generate run_tests.php that loads the extension and invokes phpunit.
        files.push(GeneratedFile {
            path: output_base.join("run_tests.php"),
            content: render_run_tests_php(&extension_name, config.php_cargo_crate_name()),
            generated_header: true,
        });

        // Generate test files per category.
        let tests_base = output_base.join("tests");
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

    crate::template_env::render(
        "php/composer.json.jinja",
        minijinja::context! {
            e2e_pkg_name => e2e_pkg_name,
            e2e_autoload_ns => e2e_autoload_ns,
            require_section => require_section,
            autoload_section => autoload_section,
        },
    )
}

fn render_phpunit_xml() -> String {
    crate::template_env::render("php/phpunit.xml.jinja", minijinja::context! {})
}

fn render_bootstrap(pkg_path: &str, has_http_fixtures: bool, has_file_fixtures: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "php/bootstrap.php.jinja",
        minijinja::context! {
            header => header,
            pkg_path => pkg_path,
            has_http_fixtures => has_http_fixtures,
            has_file_fixtures => has_file_fixtures,
        },
    )
}

fn render_run_tests_php(extension_name: &str, cargo_crate_name: Option<&str>) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let ext_lib_name = if let Some(crate_name) = cargo_crate_name {
        // Cargo replaces hyphens with underscores for lib names, and the crate name
        // already includes the _php suffix.
        format!("lib{}", crate_name.replace('-', "_"))
    } else {
        format!("lib{extension_name}_php")
    };
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

// If the locally-built extension exists and we have not already restarted with it,
// re-exec PHP with no system ini (-n) to avoid conflicts with any system-installed
// version of the extension, then load the local build explicitly.
if (file_exists($extPath) && !getenv('ALEF_PHP_LOCAL_EXT_LOADED')) {{
    putenv('ALEF_PHP_LOCAL_EXT_LOADED=1');
    $php = PHP_BINARY;
    $phpunitPath = __DIR__ . '/vendor/bin/phpunit';

    $cmd = array_merge(
        [$php, '-n', '-d', 'extension=' . $extPath],
        [$phpunitPath],
        array_slice($GLOBALS['argv'], 1)
    );

    passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);
    exit($exitCode);
}}

// Extension is now loaded (via the restart above with -n flag).
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
    let header = hash::header(CommentStyle::DoubleSlash);

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
            let element_types: Vec<String> = call
                .args
                .iter()
                .filter_map(|a| a.element_type.as_ref().map(|t| t.to_string()))
                .filter(|t| !is_php_reserved_type(t))
                .collect();
            opt_type.map(|t| t.to_string()).into_iter().chain(element_types)
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    options_type_imports.sort();

    // Build imports_use list
    let mut imports_use: Vec<String> = Vec::new();
    if needs_crawl_config_import {
        imports_use.push(format!("use {namespace}\\CrawlConfig;"));
    }
    for type_name in &options_type_imports {
        if type_name != class_name {
            imports_use.push(format!("use {namespace}\\{type_name};"));
        }
    }

    // Render all test methods
    let mut fixtures_body = String::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_method(&mut fixtures_body, fixture, fixture.http.as_ref().unwrap());
        } else {
            render_test_method(
                &mut fixtures_body,
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
            fixtures_body.push('\n');
        }
    }

    crate::template_env::render(
        "php/test_file.jinja",
        minijinja::context! {
            header => header,
            namespace => namespace,
            class_name => class_name,
            test_class => test_class,
            category => category,
            imports_use => imports_use,
            has_http_tests => has_http_tests,
            fixtures_body => fixtures_body,
        },
    )
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
        let escaped_reason = skip_reason.map(escape_php);
        let rendered = crate::template_env::render(
            "php/http_test_open.jinja",
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
        let rendered = crate::template_env::render("php/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
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

        let rendered = crate::template_env::render(
            "php/http_request.jinja",
            minijinja::context! {
                method => method,
                path => path_lit,
                opts => opts,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `$this->assertEquals(status, $response->getStatusCode())`.
    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let rendered = crate::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => status,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a header assertion using `$response->getHeaderLine(...)` or
    /// `$response->hasHeader(...)`.
    ///
    /// Handles special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_key_lit = format!("\"{}\"", escape_php(&header_key));
        let assertion_code = match expected {
            "<<present>>" => {
                format!("$this->assertTrue($response->hasHeader({header_key_lit}));")
            }
            "<<absent>>" => {
                format!("$this->assertFalse($response->hasHeader({header_key_lit}));")
            }
            "<<uuid>>" => {
                format!(
                    "$this->assertMatchesRegularExpression('/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i', $response->getHeaderLine({header_key_lit}));"
                )
            }
            literal => {
                let val_lit = format!("\"{}\"", escape_php(literal));
                format!("$this->assertEquals({val_lit}, $response->getHeaderLine({header_key_lit}));")
            }
        };

        let mut headers = vec![std::collections::HashMap::new()];
        headers[0].insert("assertion_code", assertion_code);

        let rendered = crate::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => 0u16,
                headers => headers,
                body_assertion => String::new(),
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit a JSON body equality assertion.
    ///
    /// Plain string bodies are compared against `(string) $response->getBody()` directly;
    /// structured bodies (objects, arrays, booleans, numbers) are decoded via `json_decode`
    /// and compared with `assertEquals`.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        let body_assertion = match expected {
            serde_json::Value::String(s) if !s.is_empty() => {
                let php_val = format!("\"{}\"", escape_php(s));
                format!("$this->assertEquals({php_val}, (string) $response->getBody());")
            }
            _ => {
                let php_val = json_to_php(expected);
                format!(
                    "$body = json_decode((string) $response->getBody(), true, 512, JSON_THROW_ON_ERROR);\n        $this->assertEquals({php_val}, $body);"
                )
            }
        };

        let rendered = crate::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
                status_code => 0u16,
                headers => Vec::<std::collections::HashMap<&str, String>>::new(),
                body_assertion => body_assertion,
                partial_body => Vec::<std::collections::HashMap<&str, String>>::new(),
                validation_errors => Vec::<std::collections::HashMap<&str, String>>::new(),
            },
        );
        out.push_str(&rendered);
    }

    /// Emit partial body assertions: one `assertEquals` per field in `expected`.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let mut partial_body: Vec<std::collections::HashMap<&str, String>> = Vec::new();
            for (key, val) in obj {
                let php_key = format!("\"{}\"", escape_php(key));
                let php_val = json_to_php(val);
                let assertion_code = format!("$this->assertEquals({php_val}, $body[{php_key}]);");
                let mut entry = std::collections::HashMap::new();
                entry.insert("assertion_code", assertion_code);
                partial_body.push(entry);
            }

            let rendered = crate::template_env::render(
                "php/http_assertions.jinja",
                minijinja::context! {
                    response_var => "",
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

    /// Emit validation-error assertions, checking each expected `msg` against the
    /// JSON-encoded body string (PHP binding returns ProblemDetails with `errors` array).
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let mut validation_errors: Vec<std::collections::HashMap<&str, String>> = Vec::new();
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_php(&err.msg));
            let assertion_code =
                format!("$this->assertStringContainsString({msg_lit}, json_encode($body, JSON_UNESCAPED_SLASHES));");
            let mut entry = std::collections::HashMap::new();
            entry.insert("assertion_code", assertion_code);
            validation_errors.push(entry);
        }

        let rendered = crate::template_env::render(
            "php/http_assertions.jinja",
            minijinja::context! {
                response_var => "",
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
        out.push_str(&crate::template_env::render(
            "php/http_test_skip_101.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
            },
        ));
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
    // ext-php-rs binds async Rust methods with an `_async` suffix (mirroring Magnus).
    // Append it before camelCasing so e.g. `chat` becomes `chatAsync`.
    if !has_override && call_config.r#async && !function_name.ends_with("_async") {
        function_name = format!("{function_name}_async");
    }
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

    // Check for skip_languages early
    let skip_test = call_config.skip_languages.iter().any(|l| l == "php");
    if skip_test {
        let rendered = crate::template_env::render(
            "php/test_method.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
                client_factory => String::new(),
                setup_lines => Vec::<String>::new(),
                expects_error => false,
                skip_test => true,
                has_usable_assertions => false,
                call_expr => String::new(),
                result_var => result_var,
                assertions_body => String::new(),
            },
        );
        out.push_str(&rendered);
        return;
    }

    // Build visitor if present and add to setup
    let mut options_already_created = !args_str.is_empty() && args_str == "$options";
    if let Some(visitor_spec) = &fixture.visitor {
        build_php_visitor(&mut setup_lines, visitor_spec);
        if !options_already_created {
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

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_factory = if let Some(factory) = php_client_factory {
        let fixture_id = &fixture.id;
        if has_mock {
            format!(
                "$client = \\{namespace}\\{class_name}::{factory}('test-key', getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}');"
            )
        } else if let Some(var) = api_key_var {
            format!(
                "$apiKey = getenv('{var}');\n        if (!$apiKey) {{ $this->markTestSkipped('{var} not set'); return; }}\n        $client = \\{namespace}\\{class_name}::{factory}($apiKey);"
            )
        } else {
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key');")
        }
    } else {
        String::new()
    };

    // Determine if there are usable assertions
    let has_usable_assertions = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "error" || a.assertion_type == "not_error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    // Render assertions_body
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            call_config.result_is_array,
        );
    }

    let rendered = crate::template_env::render(
        "php/test_method.jinja",
        minijinja::context! {
            method_name => method_name,
            description => description,
            client_factory => client_factory,
            setup_lines => setup_lines,
            expects_error => expects_error,
            skip_test => fixture.assertions.is_empty(),
            has_usable_assertions => has_usable_assertions,
            call_expr => call_expr,
            result_var => result_var,
            assertions_body => assertions_body,
        },
    );
    out.push_str(&rendered);
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

    // True when any arg after `from_idx` has a fixture value (or has no fixture
    // value but is required — i.e. would emit *something*). Used to decide
    // whether a missing optional middle arg must emit `null` to preserve the
    // positional argument layout, or can be safely dropped.
    let arg_has_emission = |arg: &crate::config::ArgMapping| -> bool {
        let val = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) => !arg.optional,
            Some(_) => true,
        }
    };
    let any_later_has_emission = |from_idx: usize| -> bool { args[from_idx..].iter().any(arg_has_emission) };

    for (idx, arg) in args.iter().enumerate() {
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
            let config_value = if arg.field == "input" {
                input
            } else {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                input.get(field).unwrap_or(&serde_json::Value::Null)
            };
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("${} = {class_name}::{constructor_name}(null);", arg.name,));
            } else {
                let name = &arg.name;
                // Use CrawlConfig::from_json() instead of direct property assignment.
                // ext-php-rs doesn't support writable #[php(prop)] fields for complex types,
                // so serialize the config to JSON and use from_json() to construct it.
                // Filter out empty string enum values before passing to from_json().
                let filtered_config = filter_empty_enum_strings(config_value);
                setup_lines.push(format!(
                    "${name}_config = CrawlConfig::from_json(json_encode({}));",
                    json_to_php(&filtered_config)
                ));
                setup_lines.push(format!(
                    "${} = {class_name}::{constructor_name}(${name}_config);",
                    arg.name,
                ));
            }
            parts.push(format!("${}", arg.name));
            continue;
        }

        let val = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };

        // Bytes args: fixture stores either a fixture-relative path string (load
        // with file_get_contents at runtime, mirroring the go/python convention)
        // or an inline byte array (encode as a "\xNN" escape string).
        if arg.arg_type == "bytes" {
            match val {
                None | Some(serde_json::Value::Null) => {
                    if arg.optional {
                        parts.push("null".to_string());
                    } else {
                        parts.push("\"\"".to_string());
                    }
                }
                Some(serde_json::Value::String(s)) => {
                    let var_name = format!("{}Bytes", arg.name);
                    setup_lines.push(format!(
                        "${var_name} = file_get_contents(\"{path}\");\n        if (${var_name} === false) {{ $this->fail(\"failed to read fixture: {path}\"); }}",
                        path = s.replace('"', "\\\"")
                    ));
                    parts.push(format!("${var_name}"));
                }
                Some(serde_json::Value::Array(arr)) => {
                    let bytes: String = arr
                        .iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| format!("\\x{:02x}", n))
                        .collect();
                    parts.push(format!("\"{bytes}\""));
                }
                Some(other) => {
                    parts.push(json_to_php(other));
                }
            }
            continue;
        }

        match val {
            None | Some(serde_json::Value::Null) if arg.arg_type == "json_object" && arg.name == "config" => {
                // Special case: ExtractionConfig and similar config objects with no fixture value
                // should default to an empty instance (e.g., ExtractionConfig::from_json('{}'))
                // to satisfy required parameters. This check happens BEFORE the optional check
                // so that config args are always provided, even if marked optional in alef.toml.
                // Infer the type name from the arg name and capitalize it (e.g., "config" -> "ExtractionConfig").
                let type_name = if arg.name == "config" {
                    "ExtractionConfig".to_string()
                } else {
                    format!("{}Config", arg.name.to_upper_camel_case())
                };
                parts.push(format!("{type_name}::from_json('{{}}')"));
                continue;
            }
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value. If a later arg WILL emit
                // something, we must keep this slot in place by passing `null`
                // so the positional argument layout matches the PHP signature.
                // Otherwise drop the trailing optional argument entirely.
                if any_later_has_emission(idx + 1) {
                    parts.push("null".to_string());
                }
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
                        // When element_type is a scalar/primitive and value is an array,
                        // pass it directly as a PHP array (e.g. ["python"]) rather than
                        // wrapping in a typed config constructor.
                        if v.is_array() && is_php_reserved_type(elem_type) {
                            parts.push(json_to_php(v));
                            continue;
                        }
                    }
                    match options_via {
                        "json" => {
                            // Pass as JSON string via json_encode(); the Rust method accepts Option<String>.
                            // Filter out empty string enum values.
                            let filtered_v = filter_empty_enum_strings(v);

                            // If the config is empty after filtering, pass null instead.
                            if let serde_json::Value::Object(obj) = &filtered_v {
                                if obj.is_empty() {
                                    parts.push("null".to_string());
                                    continue;
                                }
                            }

                            parts.push(format!("json_encode({})", json_to_php_camel_keys(&filtered_v)));
                            continue;
                        }
                        _ => {
                            if let Some(type_name) = options_type {
                                // Use TypeName::from_json(json_encode([...])) to construct the
                                // typed config object. ext-php-rs structs expose a from_json()
                                // static method that accepts a JSON string.
                                // Filter out empty string enum values before passing to from_json().
                                let filtered_v = filter_empty_enum_strings(v);

                                // For empty objects, construct with from_json('{}') to get the
                                // type's defaults rather than passing null (which fails for non-optional params).
                                if let serde_json::Value::Object(obj) = &filtered_v {
                                    if obj.is_empty() {
                                        let arg_var = format!("${}", arg.name);
                                        setup_lines.push(format!("{arg_var} = {type_name}::from_json('{{}}');"));
                                        parts.push(arg_var);
                                        continue;
                                    }
                                }

                                let arg_var = format!("${}", arg.name);
                                // Use json_to_php (snake_case) instead of json_to_php_camel_keys because
                                // Rust's serde deserializes field names as snake_case by default (via #[serde(rename_all = "snake_case")]).
                                // PHP should match Rust field naming conventions, not use camelCase.
                                setup_lines.push(format!(
                                    "{arg_var} = {type_name}::from_json(json_encode({}));",
                                    json_to_php(&filtered_v)
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
    result_is_array: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->content), true)"
                );
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_content",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->embedding), true)"
                );
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns array<array<float>> in PHP — no wrapper object.
            // $result_var is the embedding matrix; use it directly.
            "embeddings" => {
                let php_val = assertion.value.as_ref().map(json_to_php).unwrap_or_default();
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        php_val => php_val,
                        result_var => result_var,
                    },
                ));
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(empty(${result_var}) ? 0 : count(${result_var}[0]))");
                let php_val = assertion.value.as_ref().map(json_to_php).unwrap_or_default();
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embedding_dimensions",
                        assertion_type => assertion.assertion_type.as_str(),
                        expr => expr,
                        php_val => php_val,
                    },
                ));
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
                let assertion_kind = format!("embeddings_{}", f.strip_prefix("embeddings_").unwrap_or(f));
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => assertion_kind,
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- keywords / keywords_count ----
            // PHP ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "keywords",
                        field_name => f,
                    },
                ));
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            out.push_str(&crate::template_env::render(
                "php/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
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
                out.push_str(&crate::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "result_is_simple",
                        field_name => f,
                    },
                ));
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

    // Detect if this field is an array type
    // When there's no field, default to result_is_array (the result itself is the array)
    let field_is_array = assertion.field.as_ref().map_or(result_is_array, |f| {
        if f.is_empty() {
            result_is_array
        } else {
            field_resolver.is_array(f)
        }
    });

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

    // Prepare template context.
    let assertion_type = assertion.assertion_type.as_str();
    let has_php_val = assertion.value.is_some();
    // serde collapses `"value": null` to `None`, but `equals` against null is a real
    // assertion (e.g. `result.message.content == null`). Default to PHP `null` in that
    // case so the rendered code compiles instead of producing `assertEquals(, ...)`.
    let php_val = match assertion.value.as_ref() {
        Some(v) => json_to_php(v),
        None if assertion_type == "equals" => "null".to_string(),
        None => String::new(),
    };
    let trimmed_field_expr = trimmed_field_expr_for(assertion.value.as_ref().unwrap_or(&serde_json::Value::Null));
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let values_php: Vec<String> = assertion
        .values
        .as_ref()
        .map_or(Vec::new(), |vals| vals.iter().map(json_to_php).collect());
    let contains_any_checks: Vec<String> = assertion
        .values
        .as_ref()
        .map_or(Vec::new(), |vals| vals.iter().map(json_to_php).collect());
    let n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    // For method_result assertions.
    let call_expr = if let Some(method_name) = &assertion.method {
        build_php_method_call(result_var, method_name, assertion.args.as_ref())
    } else {
        String::new()
    };
    let check = assertion.check.as_deref().unwrap_or("is_true");
    let has_php_check_val = matches!(assertion.assertion_type.as_str(), "method_result") && assertion.value.is_some();
    let php_check_val = if matches!(assertion.assertion_type.as_str(), "method_result") {
        assertion.value.as_ref().map(json_to_php).unwrap_or_default()
    } else {
        String::new()
    };
    let check_n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);
    let is_bool_val = assertion.value.as_ref().is_some_and(|v| v.is_boolean());
    let bool_is_true = assertion.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);

    // Early returns for non-template-renderable assertions.
    if matches!(assertion_type, "not_error" | "error") {
        if assertion_type == "not_error" {
            // Already handled by the call succeeding without exception.
        }
        // "error" is handled at the test method level.
        return;
    }

    let rendered = crate::template_env::render(
        "php/assertion.jinja",
        minijinja::context! {
            assertion_type => assertion_type,
            field_expr => field_expr,
            php_val => php_val,
            has_php_val => has_php_val,
            trimmed_field_expr => trimmed_field_expr,
            is_string_val => is_string_val,
            field_is_array => field_is_array,
            values_php => values_php,
            contains_any_checks => contains_any_checks,
            n => n,
            call_expr => call_expr,
            check => check,
            php_check_val => php_check_val,
            has_php_check_val => has_php_check_val,
            check_n => check_n,
            is_bool_val => is_bool_val,
            bool_is_true => bool_is_true,
        },
    );
    let _ = write!(out, "        {}", rendered);
}

/// Build a PHP call expression for a `method_result` assertion.
///
/// Uses generic instance method dispatch: `$result_var->method_name(args...)`.
/// Args from the fixture JSON object are emitted as positional PHP arguments in
/// insertion order, using best-effort type conversion (strings → PHP string literals,
/// numbers and booleans → verbatim literals).
fn build_php_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
                        serde_json::Value::Bool(true) => "true".to_string(),
                        serde_json::Value::Bool(false) => "false".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "null".to_string(),
                        other => format!("\"{}\"", other.to_string().replace('\\', "\\\\").replace('"', "\\\"")),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("${result_var}->{method_name}()")
    } else {
        format!("${result_var}->{method_name}({extra_args})")
    }
}

/// Filters out empty string enum values from JSON objects before rendering.
/// When a field has an empty string value, it's treated as a missing/null enum field
/// and should not be included in the PHP array.
fn filter_empty_enum_strings(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter_map(|(k, v)| {
                    // Skip empty string values (typically represent missing enum variants)
                    if let serde_json::Value::String(s) = v {
                        if s.is_empty() {
                            return None;
                        }
                    }
                    // Recursively filter nested objects and arrays
                    Some((k.clone(), filter_empty_enum_strings(v)))
                })
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let filtered: Vec<serde_json::Value> = arr.iter().map(filter_empty_enum_strings).collect();
            serde_json::Value::Array(filtered)
        }
        other => other.clone(),
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

/// Like `json_to_php` but recursively converts all object keys to lowerCamelCase.
/// Used when generating PHP option arrays passed to `from_json()` — the PHP binding
/// structs use `#[serde(rename_all = "camelCase")]` so snake_case fixture keys
/// (e.g. `remove_forms`) must become `removeForms` in the generated test code.
fn json_to_php_camel_keys(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let camel_key = k.to_lower_camel_case();
                    format!("\"{}\" => {}", escape_php(&camel_key), json_to_php_camel_keys(v))
                })
                .collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_php_camel_keys).collect();
            format!("[{}]", items.join(", "))
        }
        _ => json_to_php(value),
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
        "visit_input" => "$ctx, $input_type, $name, $value",
        "visit_audio" | "visit_video" | "visit_iframe" => "$ctx, $src",
        "visit_details" => "$ctx, $isOpen",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "$ctx, $output",
        "visit_list_start" => "$ctx, $ordered",
        "visit_list_end" => "$ctx, $ordered, $output",
        _ => "$ctx",
    };

    let (action_type, action_value) = match action {
        CallbackAction::Skip => ("skip", String::new()),
        CallbackAction::Continue => ("continue", String::new()),
        CallbackAction::PreserveHtml => ("preserve_html", String::new()),
        CallbackAction::Custom { output } => ("custom", escape_php(output)),
        CallbackAction::CustomTemplate { template } => ("custom_template", escape_php(template)),
    };

    let rendered = crate::template_env::render(
        "php/visitor_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params,
            action_type => action_type,
            action_value => action_value,
        },
    );
    for line in rendered.lines() {
        setup_lines.push(line.to_string());
    }
}

/// Returns true if the type name is a PHP reserved/primitive type that cannot be imported.
fn is_php_reserved_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "string"
            | "int"
            | "integer"
            | "float"
            | "double"
            | "bool"
            | "boolean"
            | "array"
            | "object"
            | "null"
            | "void"
            | "callable"
            | "iterable"
            | "never"
            | "self"
            | "parent"
            | "static"
            | "true"
            | "false"
            | "mixed"
    )
}
