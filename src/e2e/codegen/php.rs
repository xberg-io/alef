//! PHP e2e test generator using PHPUnit.
//!
//! Generates `e2e/php/composer.json`, `e2e/php/phpunit.xml`, and
//! `tests/{Category}Test.php` files from JSON fixtures, driven entirely by
//! `E2eConfig` and `CallConfig`.

use crate::backends::php::naming::php_autoload_namespace;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::TypeRef;
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_php, sanitize_filename};
use crate::e2e::field_access::{FieldResolver, PhpGetterMap};
use crate::e2e::fixture::{
    Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture, TemplateReturnForm, ValidationErrorExpectation,
};
use anyhow::Result;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::collections::{HashMap, HashSet};
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
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
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
                    .and_then(crate::core::config::derive_repo_org)
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
                &extension_name,
                &pkg_name,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
            ),
            generated_header: false,
        });

        // Generate install.sh (registry mode only) — bootstraps PIE and installs
        // the extension before `composer install` runs in the verify-install flow.
        // The pinned version is baked in at generate time so callers can run
        // `bash install.sh` with no args.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            files.push(GeneratedFile {
                path: output_base.join("install.sh"),
                content: render_install_sh(&pkg_name, &extension_name, &pkg_version),
                generated_header: false,
            });
        }

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
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Generate bootstrap.php that loads both autoloaders and optionally starts the mock server.
        files.push(GeneratedFile {
            path: output_base.join("bootstrap.php"),
            content: render_bootstrap(
                &pkg_path,
                has_http_fixtures,
                has_file_fixtures,
                &e2e_config.test_documents_relative_from(0),
            ),
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

        // Compute per-(type, field) getter classification for PHP.
        // ext-php-rs 0.15.x exposes scalar fields as PHP properties via `#[php(prop)]`,
        // but non-scalar fields (Named structs, Vec<Named>, Map, etc.) need a
        // `#[php(getter)]` method because `get_method_props` is `todo!()` in
        // ext-php-rs-derive 0.11.7. E2e assertions must call `->getCamelCase()` for those.
        //
        // The classification MUST be keyed by (owner_type, field_name) rather than
        // bare field_name: two unrelated types can declare the same field name with
        // different scalarness (e.g. `CrawlConfig.content: ContentConfig` vs
        // `MarkdownResult.content: String`). A bare-name union would force every
        // `->content` access to `->getContent()` even on types where it is a scalar
        // property — see kreuzcrawl regression where `MarkdownResult::getContent()`
        // does not exist.
        let php_enum_names: HashSet<String> = enums.iter().map(|e| e.name.clone()).collect();

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
            let php_lang_rename_all = config.serde_rename_all_for_language(crate::core::config::Language::Php);
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                lang,
                &namespace,
                &class_name,
                &test_class,
                type_defs,
                &php_enum_names,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
                &config.adapters,
                php_lang_rename_all,
                config,
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
// PHP scalar-type predicate
// ---------------------------------------------------------------------------

/// Returns true when a type is scalar-compatible for ext-php-rs `#[php(prop)]` —
/// that is, the mapped Rust type implements `IntoZval` + `FromZval` automatically
/// without a manual getter. Mirrors `is_php_prop_scalar_with_enums` from
/// `alef-backend-php/src/gen_bindings/types.rs`.
///
/// Scalar-compatible: primitives, String, Char, Duration (→ u64), Path (→ String),
/// Option<scalar>, Vec<primitive|String|Char>, unit-variant enums (mapped to String).
/// Non-scalar: Named struct, Map, nested Vec<Named>, Json, Bytes.
/// Build a per-`(owner_type, field_name)` PHP getter classification plus chain-resolution
/// metadata from the IR's `TypeDef`s.
///
/// For each type, marks fields as needing getter syntax when their mapped Rust type
/// is non-scalar in PHP (Named struct, Vec<Named>, Map, Json, Bytes). Also records each
/// field's referenced `Named` inner type so the resolver can advance the current-type
/// cursor as it walks multi-segment paths like `outer.inner.content`.
///
/// `root_type` is derived (best-effort) from a `result_type` override on any backend
/// (`c`, `csharp`, `java`, `kotlin`, `go`, `php`) and otherwise inferred by matching
/// `result_fields` against `TypeDef.fields`. When no root can be determined, chain
/// resolution falls back to the legacy bare-name union (sound only when no field names
/// collide across types).
fn build_php_getter_map(
    type_defs: &[crate::core::ir::TypeDef],
    enum_names: &HashSet<String>,
    call: &crate::core::config::e2e::CallConfig,
    result_fields: &HashSet<String>,
) -> PhpGetterMap {
    let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut all_fields: HashMap<String, HashSet<String>> = HashMap::new();
    for td in type_defs {
        let mut getter_fields: HashSet<String> = HashSet::new();
        let mut field_type_map: HashMap<String, String> = HashMap::new();
        let mut td_all_fields: HashSet<String> = HashSet::new();
        for f in &td.fields {
            td_all_fields.insert(f.name.clone());
            if !is_php_scalar(&f.ty, enum_names) {
                getter_fields.insert(f.name.clone());
            }
            if let Some(named) = inner_named(&f.ty) {
                field_type_map.insert(f.name.clone(), named);
            }
        }
        getters.insert(td.name.clone(), getter_fields);
        all_fields.insert(td.name.clone(), td_all_fields);
        if !field_type_map.is_empty() {
            field_types.insert(td.name.clone(), field_type_map);
        }
    }
    let root_type = derive_root_type(call, type_defs, result_fields);
    PhpGetterMap {
        getters,
        field_types,
        root_type,
        all_fields,
    }
}

/// Unwrap `Option<T>` / `Vec<T>` to the innermost `Named` type name, if any.
/// Returns `None` for primitives, scalars, Map, Json, Bytes, and Unit.
fn inner_named(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(n) => Some(n.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
        _ => None,
    }
}

/// Derive the IR type name backing the result variable in PHP-generated assertions.
///
/// Lookup order:
/// 1. `call.overrides[<lang>].result_type` for any of `php`, `c`, `csharp`, `java`,
///    `kotlin`, `go` (first non-empty wins).
/// 2. Type-defs whose field names form a superset of `result_fields` (when exactly
///    one matches).
///
/// Returns `None` when neither yields a definitive answer; callers fall back to the
/// legacy bare-name union behaviour.
fn derive_root_type(
    call: &crate::core::config::e2e::CallConfig,
    type_defs: &[crate::core::ir::TypeDef],
    result_fields: &HashSet<String>,
) -> Option<String> {
    const LOOKUP_LANGS: &[&str] = &["php", "c", "csharp", "java", "kotlin", "go"];
    for lang in LOOKUP_LANGS {
        if let Some(o) = call.overrides.get(*lang)
            && let Some(rt) = o.result_type.as_deref()
            && !rt.is_empty()
            && type_defs.iter().any(|td| td.name == rt)
        {
            return Some(rt.to_string());
        }
    }
    if result_fields.is_empty() {
        return None;
    }
    let matches: Vec<&crate::core::ir::TypeDef> = type_defs
        .iter()
        .filter(|td| {
            let names: HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
            result_fields.iter().all(|rf| names.contains(rf.as_str()))
        })
        .collect();
    if matches.len() == 1 {
        return Some(matches[0].name.clone());
    }
    None
}

fn is_php_scalar(ty: &TypeRef, enum_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char | TypeRef::Duration | TypeRef::Path => true,
        TypeRef::Optional(inner) => is_php_scalar(inner, enum_names),
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char)
                || matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n))
        }
        TypeRef::Named(n) if enum_names.contains(n) => true,
        TypeRef::Named(_) | TypeRef::Map(_, _) | TypeRef::Json | TypeRef::Bytes | TypeRef::Unit => false,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_composer_json(
    e2e_pkg_name: &str,
    e2e_autoload_ns: &str,
    extension_name: &str,
    pkg_name: &str,
    pkg_path: &str,
    _pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let (require_section, autoload_section) = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // For `type: php-ext` packages the canonical consumer shape (mirroring
            // asgrim/example-pie-extension) declares a platform require on
            // `ext-<name>: "*"` rather than a direct Composer package require.
            // Composer's platform resolver checks `php -m` for the extension and
            // treats the require as satisfied once PIE has installed the .so.
            //
            // A direct `"{pkg_name}": "{pkg_version}"` require fails because
            // Composer's resolver cross-checks `ext-<name>` against `php -m` and
            // emits "found but not loaded, likely conflicts" when the extension
            // hasn't been loaded yet — before `pie install` has run.
            //
            // `minimum-stability: "dev"` / `prefer-stable: true` are no longer
            // needed: the platform require `ext-<name>: "*"` carries no stability
            // constraint, and `php: ">=8.2"` is a platform constraint that is always
            // satisfied on CI runners. The PIE install step (install.sh) is
            // responsible for resolving the correct release version.
            let require = format!(
                r#"  "require": {{
    "php": ">=8.2",
    "ext-{extension_name}": "*"
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
        crate::e2e::config::DependencyMode::Local => {
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

    crate::e2e::template_env::render(
        "php/composer.json.jinja",
        minijinja::context! {
            e2e_pkg_name => e2e_pkg_name,
            e2e_autoload_ns => e2e_autoload_ns,
            require_section => require_section,
            autoload_section => autoload_section,
        },
    )
}

/// Render the `install.sh` script placed next to `composer.json` in the
/// registry-mode PHP test_app.
///
/// The script bootstraps `php/pie` globally (if absent or older than 1.3.7),
/// runs `pie install <pkg>:<version>`, and verifies the extension binary loads.
/// The pinned version is baked in at generate time; callers run
/// `bash install.sh` with no arguments. The default `alef test-apps run`
/// command for PHP invokes this script before `composer install`.
/// Strip leading composer-style version constraints (^, >=, ~, etc.) from a version string.
/// Accepts "1.2.3", ">=1.2.3", "^1.2.3", "~1.2", or any constraint and returns the base version.
fn strip_version_constraint(version: &str) -> &str {
    version
        .trim_start_matches(|c: char| c == '^' || c == '~' || c == '>' || c == '<' || c == '=')
}

fn render_install_sh(pkg_name: &str, extension_name: &str, pkg_version: &str) -> String {
    let clean_version = strip_version_constraint(pkg_version);
    format!(
        r#"#!/usr/bin/env bash
# alef-generated installer for registry-mode PHP test_app.
# Installs the {pkg_name} extension via PIE before `composer install` runs.
# Requires `php` on PATH; downloads and runs PIE if needed.
# Version is alef-injected at generate time so the script is self-contained.
set -euo pipefail

# Version override: pass as $1 to test an arbitrary tag; defaults to the
# alef-pinned version from `[crates.e2e.registry.packages.php].version`.
VERSION="${{1:-{clean_version}}}"

# PIE >= 1.3.7 supports the array-form `php-ext.download-url-method`
# our composer.json emits; 1.4.0+ is preferred. Download PIE if we don't
# already have a recent enough version.
need_pie_install=true
if command -v pie >/dev/null 2>&1; then
  current="$(pie --version 2>&1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo '0.0.0')"
  if printf '%s\n%s\n' "1.3.7" "$current" | sort -V -C; then
    need_pie_install=false
  fi
fi
if [[ "$need_pie_install" == "true" ]]; then
  # Download PIE PHAR from latest GitHub release if not already installed.
  pie_dir="${{HOME}}/.local/bin"
  mkdir -p "$pie_dir"
  curl -fL --output "$pie_dir/pie" "https://github.com/php/pie/releases/latest/download/pie.phar" 2>/dev/null || {{
    echo "::error::Failed to download PIE from GitHub; ensure network access or pre-install PIE." >&2
    exit 1
  }}
  chmod +x "$pie_dir/pie"
  PIE="$pie_dir/pie"
  # Ensure newly downloaded PIE is on PATH for this script.
  export PATH="$pie_dir:$PATH"
else
  PIE="pie"
fi

# Install the extension binary into the running PHP's extension dir.
"$PIE" install "{pkg_name}:$VERSION" --skip-enable-extension

# Verify the .so loads.
EXT_DIR="$(php -r 'echo ini_get("extension_dir");')"
test -f "$EXT_DIR/{extension_name}.so" || test -f "$EXT_DIR/{extension_name}.dylib" || test -f "$EXT_DIR/{extension_name}.dll"

# Load it explicitly for the smoke test (the verify-install action runs
# phpunit with this same `-dextension=` flag in CI).
if ! php -dextension={extension_name} -m | grep -qi {extension_name}; then
  echo "::error::{extension_name} extension failed to load after PIE install" >&2
  exit 1
fi
echo "{extension_name} extension installed and loaded"
"#
    )
}

fn render_phpunit_xml() -> String {
    crate::e2e::template_env::render("php/phpunit.xml.jinja", minijinja::context! {})
}

fn render_bootstrap(
    pkg_path: &str,
    has_http_fixtures: bool,
    has_file_fixtures: bool,
    test_documents_path: &str,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::e2e::template_env::render(
        "php/bootstrap.php.jinja",
        minijinja::context! {
            header => header,
            pkg_path => pkg_path,
            has_http_fixtures => has_http_fixtures,
            has_file_fixtures => has_file_fixtures,
            test_documents_path => test_documents_path,
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
// re-exec PHP with the freshly-built extension loaded explicitly via `-d extension=`.
// The system php.ini is kept (no `-n`) so PHPUnit's required extensions — dom, json,
// libxml, mbstring, tokenizer, xml, xmlwriter — remain available. `-n` drops every
// shared module, which breaks PHPUnit on distributions that ship those as shared
// extensions (e.g. Debian/Ubuntu); they only survive `-n` where compiled statically.
if (file_exists($extPath) && !getenv('ALEF_PHP_LOCAL_EXT_LOADED')) {{
    putenv('ALEF_PHP_LOCAL_EXT_LOADED=1');
    $php = PHP_BINARY;
    $phpunitPath = __DIR__ . '/vendor/bin/phpunit';

    $cmd = array_merge(
        [$php, '-d', 'extension=' . $extPath],
        [$phpunitPath],
        array_slice($GLOBALS['argv'], 1)
    );

    passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);
    exit($exitCode);
}}

// Extension is now loaded (via the restart above).
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
    type_defs: &[crate::core::ir::TypeDef],
    php_enum_names: &HashSet<String>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    php_lang_rename_all: String,
    config: &ResolvedCrateConfig,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    // Determine if any fixture is an HTTP test (needs GuzzleHttp).
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    // Collect options_type class names that need `use` imports (one import per unique name).
    // Also collects:
    //   - per-arg `element_type` for `Vec<T>` arguments (e.g. `PageAction`).
    //   - streaming-adapter `request_type` for fixtures that invoke a streaming
    //     adapter call (e.g. `CrawlStreamRequest`). Without this import the
    //     generated `new CrawlStreamRequest($url)` resolves to
    //     `Kreuzcrawl\E2e\CrawlStreamRequest` (the test namespace) and PHPUnit
    //     errors with `Class "...\CrawlStreamRequest" not found`.
    let mut options_type_imports: Vec<String> = fixtures
        .iter()
        .flat_map(|f| {
            let call = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            let php_override = call.overrides.get(lang);
            let opt_type = php_override
                .and_then(|o| o.options_type.as_deref())
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get(lang)
                        .and_then(|o| o.options_type.as_deref())
                })
                .or(call.options_type.as_deref());
            let element_types: Vec<String> = call
                .args
                .iter()
                .filter_map(|a| a.element_type.as_ref().map(|t| t.to_string()))
                .filter(|t| !is_php_reserved_type(t))
                .collect();
            let adapter_request_type: Option<String> = adapters
                .iter()
                .find(|a| a.name == call.function.as_str())
                .and_then(|a| a.request_type.as_deref())
                .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
            opt_type
                .map(|t| t.to_string())
                .into_iter()
                .chain(element_types)
                .chain(adapter_request_type)
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    options_type_imports.sort();

    // Build imports_use list
    let mut imports_use: Vec<String> = Vec::new();
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
                type_defs,
                php_enum_names,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
                adapters,
                &php_lang_rename_all,
                config,
            );
        }
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    crate::e2e::template_env::render(
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
        let rendered = crate::e2e::template_env::render(
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
        let rendered = crate::e2e::template_env::render("php/http_test_close.jinja", minijinja::context! {});
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

        // The template wraps `path` in double quotes itself, so emit only the escaped
        // contents here — wrapping again produces an invalid `""…""` string literal.
        let path_lit = escape_php(ctx.path);

        let rendered = crate::e2e::template_env::render(
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
        let rendered = crate::e2e::template_env::render(
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

        let rendered = crate::e2e::template_env::render(
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

        let rendered = crate::e2e::template_env::render(
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

            let rendered = crate::e2e::template_env::render(
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

        let rendered = crate::e2e::template_env::render(
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
        out.push_str(&crate::e2e::template_env::render(
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
    type_defs: &[crate::core::ir::TypeDef],
    php_enum_names: &HashSet<String>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    php_lang_rename_all: &str,
    config: &ResolvedCrateConfig,
) {
    // Resolve per-fixture call config: supports named calls via fixture.call field.
    let mut call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Fallback: if the resolved call has required args missing from input,
    // try to find a better-matching call from the named calls.
    call_config = super::select_best_matching_call(call_config, e2e_config, fixture);
    // Build per-call PHP getter map and field resolver using the effective field sets.
    let per_call_getter_map = build_php_getter_map(
        type_defs,
        php_enum_names,
        call_config,
        e2e_config.effective_result_fields(call_config),
    );
    let call_field_resolver = FieldResolver::new_with_php_getters(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
        &HashMap::new(),
        per_call_getter_map,
    );
    let field_resolver = &call_field_resolver;
    let call_overrides = call_config.overrides.get(lang);
    let has_override = call_overrides.is_some_and(|o| o.function.is_some());
    // Per-call result_is_simple override wins over the language-level default,
    // so calls like `speech` (returns Vec<u8>) can be marked simple even if
    // chat/embed are not.
    let result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple) || result_is_simple;
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // The PHP facade exposes async Rust methods under their bare name (no `_async`
    // suffix) — PHP has no surface-level async, so the facade picks the async
    // implementation as the default and delegates to `*Async` on the native class.
    // The `*_sync` variants stay explicit (e.g. `extract_bytes_sync` → `extractBytesSync`).
    if !has_override {
        function_name = function_name.to_lower_camel_case();
    }
    let result_var = &call_config.result_var;
    let args = fixture.resolved_args(call_config);

    let method_name = sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve options_type for this call. Precedence: per-language call override,
    // then the call-level `options_type` (the binding-agnostic config parameter type,
    // e.g. `EmbeddingConfig`), then the global per-language call override (fallback default).
    let call_options_type = call_overrides
        .and_then(|o| o.options_type.as_deref())
        .or(call_config.options_type.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.options_type.as_deref())
        });

    let call_adapter = adapters.iter().find(|a| a.name == call_config.function.as_str());
    let adapter_request_type: Option<String> = call_adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`$engine->crawlStream($req)`), not as static facade methods.
    // Capture the owner handle variable so the call is rendered as an
    // instance-method invocation and the handle is omitted from the argument list.
    let streaming_owner_handle: Option<String> = if call_adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    }) {
        args.iter().find(|a| a.arg_type == "handle").map(|a| a.name.clone())
    } else {
        None
    };

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        class_name,
        enum_fields,
        fixture,
        options_via,
        call_options_type,
        adapter_request_type.as_deref(),
        namespace,
        streaming_owner_handle.is_some(),
        type_defs,
        php_lang_rename_all,
        config,
    );

    // Check for skip_languages early
    let skip_test = call_config.skip_languages.iter().any(|l| l == "php");
    if skip_test {
        let rendered = crate::e2e::template_env::render(
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
            let options_type = call_options_type.unwrap_or("ConversionOptions");
            if options_via == "from_json" {
                // When options_via is "from_json", create options from JSON first,
                // then attach the visitor using with_visitor() since PHP closures can't be JSON-encoded.
                setup_lines.push(format!("$options = \\{namespace}\\{options_type}::from_json('{{}}');"));
                setup_lines.push(format!(
                    "$visitorHandle = \\{namespace}\\VisitorHandle::from_php_object($visitor);"
                ));
                // ext-php-rs camel-cases snake_case method names; the generated PHP class
                // exposes the wither as `withVisitor`, not `with_visitor`.
                setup_lines.push("$options = $options->withVisitor($visitorHandle);".to_string());
            } else {
                // Default builder pattern for other options_via modes
                setup_lines.push(format!("$builder = \\{namespace}\\{options_type}::builder();"));
                setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
            }
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
    } else if let Some(ref handle_var) = streaming_owner_handle {
        // Instance-method invocation on the owner handle.
        format!("${handle_var}->{function_name}({final_args})")
    } else {
        format!("{class_name}::{function_name}({final_args})")
    };

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_factory = if let Some(factory) = php_client_factory {
        let fixture_id = &fixture.id;
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            format!(
                "$apiKey = getenv('{var}');\n        $baseUrl = ($apiKey !== false && $apiKey !== '') ? null : getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';\n        fwrite(STDERR, \"{fixture_id}: \" . ($baseUrl === null ? 'using real API ({var} is set)' : 'using mock server ({var} not set)') . \"\\n\");\n        $client = \\{namespace}\\{class_name}::{factory}($baseUrl === null ? $apiKey : 'test-key', $baseUrl);"
            )
        } else if has_mock {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!("(getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}')")
            } else {
                format!("getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}'")
            };
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key', {base_url_expr});")
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

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming = crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming);

    // Determine if there are usable assertions.
    // For streaming fixtures: streaming virtual fields count as usable.
    let has_usable_assertions = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "error" || a.assertion_type == "not_error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => {
                if is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                    return true;
                }
                // Account for synthetic assertion fields that render_assertion handles
                let is_synthetic_field = matches!(
                    f.as_str(),
                    "chunks_have_content"
                        | "chunks_have_embeddings"
                        | "chunks_have_heading_context"
                        | "first_chunk_starts_with_heading"
                        | "embeddings"
                        | "embedding_dimensions"
                        | "embeddings_valid"
                        | "embeddings_finite"
                        | "embeddings_non_zero"
                        | "embeddings_normalized"
                );
                is_synthetic_field || field_resolver.is_valid_for_result(f)
            }
            _ => true,
        }
    });

    // For streaming fixtures, emit collect snippet after the result assignment.
    let collect_snippet = if is_streaming {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet("php", result_var, "chunks")
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Collect fields_array fields that are referenced in assertions
    // so we can emit bindings for them (e.g., $chunks = $result->getChunks();).
    //
    // Use a BTreeMap (sorted by key) so the emitted accessor extraction lines
    // appear in a stable order across regens. A HashMap here previously leaked
    // its randomized iteration order into the generated PHP source, causing
    // e.g. tslp's `e2e/php/tests/ProcessTest.php` to flip the relative order
    // of `$imports` vs `$structure` bindings between back-to-back
    // `alef e2e generate` invocations.
    let mut fields_array_bindings: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            // Skip enum variant accessor paths (metadata.format.excel etc.)
            let is_enum_variant_accessor = f.contains("metadata.format.") && f.matches('.').count() >= 2;
            if !f.is_empty()
                && !is_enum_variant_accessor
                && field_resolver.is_array(f)
                // Only collect bindings for fields that are valid on the result type
                && field_resolver.is_valid_for_result(f)
            {
                // Only emit binding if not already added
                if !fields_array_bindings.contains_key(f.as_str()) {
                    let accessor = field_resolver.accessor(f, "php", &format!("${result_var}"));
                    let var_name = f.to_lower_camel_case();
                    fields_array_bindings.insert(f.clone(), (var_name, accessor));
                }
            }
        }
    }

    // Generate field binding lines (e.g., $chunks = $result->getChunks();)
    // Every collected array-binding accessor needs its $var emitted; the prior
    // hardcoded allowlist ("chunks"/"imports"/"structure") silently dropped
    // bindings like $choices0MessageToolCalls and $segments, leaving
    // assertions that reference them to fail with "Undefined variable".
    // BTreeMap iteration is sorted-by-key, so this loop is deterministic.
    let mut field_bindings = String::new();
    for (var_name, accessor) in fields_array_bindings.values() {
        field_bindings.push_str(&format!("        ${} = {};\n", var_name, accessor));
    }

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
            &fields_array_bindings,
            is_streaming,
        );
    }

    // Streaming fixtures whose only assertion is `not_error` produce an empty
    // assertions_body even though the stream were drained successfully.  PHPUnit
    // flags such tests as "risky" (no assertions performed).  Emit a minimal
    // structural assertion against the drained chunk list so the test records
    // success without false-positive reliance on `expectNotToPerformAssertions`.
    if is_streaming && !expects_error && assertions_body.trim().is_empty() {
        assertions_body.push_str("        $this->assertTrue(is_array($chunks), 'expected drained chunks list');\n");
    }

    let rendered = crate::e2e::template_env::render(
        "php/test_method.jinja",
        minijinja::context! {
            method_name => method_name,
            description => description,
            client_factory => client_factory,
            setup_lines => setup_lines,
            expects_error => expects_error,
            skip_test => fixture.assertions.is_empty(),
            has_usable_assertions => has_usable_assertions || is_streaming,
            call_expr => call_expr,
            result_var => result_var,
            collect_snippet => collect_snippet,
            field_bindings => field_bindings,
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
                        _ => {
                            // Generic handling for tagged-enum DTOs (PageAction, etc.): wrap each
                            // element in `{Type}::from_json('{...}')`. The PHP binding's #[php_class]
                            // FromZval only accepts class instances; raw PHP arrays are rejected with
                            // "Failed to convert array element to {Type}". Every alef-emitted class
                            // with a tagged-enum shape (or any has_default struct) has a `from_json`
                            // static method generated from `serde::Deserialize`, so this is the
                            // portable path that works without a per-class FromZval impl.
                            let json_str = serde_json::to_string(&serde_json::Value::Object(obj.clone()))
                                .unwrap_or_else(|_| "{}".to_string());
                            let php_literal = json_str.replace('\\', "\\\\").replace('\'', "\\'");
                            Some(format!("{}::from_json('{}')", elem_type, php_literal))
                        }
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

#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    class_name: &str,
    _enum_fields: &HashMap<String, String>,
    fixture: &crate::e2e::fixture::Fixture,
    options_via: &str,
    options_type: Option<&str>,
    adapter_request_type: Option<&str>,
    namespace: &str,
    owner_handle_is_receiver: bool,
    type_defs: &[crate::core::ir::TypeDef],
    php_lang_rename_all: &str,
    config: &ResolvedCrateConfig,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
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
    let arg_has_emission = |arg: &crate::e2e::config::ArgMapping| -> bool {
        let val = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) => {
                // A `json_object` arg named `config` always emits a value (a default
                // `Type::from_json('{}')`) regardless of `optional`, mirroring the
                // unconditional special case in the per-arg loop below. Treating it as
                // "no emission" would let an earlier optional arg (e.g. `mime_type`) be
                // dropped, shifting `config` into the wrong positional slot.
                if arg.arg_type == "json_object" && arg.name == "config" {
                    return true;
                }
                !arg.optional
            }
            Some(_) => true,
        }
    };
    let any_later_has_emission = |from_idx: usize| -> bool { args[from_idx..].iter().any(arg_has_emission) };

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "${} = getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "${} = getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("${}_req", arg.name);
                setup_lines.push(format!("{req_var} = new {req_type}(${});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(format!("${}", arg.name));
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // array of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let val = if let Some(v) = input.get(field).filter(|v| !v.is_null()) {
                v.clone()
            } else {
                super::resolve_urls_field(input, &arg.field).clone()
            };
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_php(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "${name}_base = getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';"
            ));
            setup_lines.push(format!(
                "${name} = array_map(fn($p) => str_starts_with($p, 'http') ? $p : ${name}_base . $p, [{paths_literal}]);"
            ));
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("${name}_req");
                setup_lines.push(format!("{req_var} = new {req_type}(${name});"));
                parts.push(req_var);
            } else {
                parts.push(format!("${name}"));
            }
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
                // Use <ConfigType>::from_json() instead of direct property assignment.
                // ext-php-rs doesn't support writable #[php(prop)] fields for complex types,
                // so serialize the config to JSON and use from_json() to construct it.
                // Filter out empty string enum values before passing to from_json().
                let filtered_config = filter_empty_enum_strings(config_value);
                // The PHP binding's `from_json` deserializes into the binding struct, which is
                // always emitted with `#[serde(rename_all = "{php_lang_rename_all}")]` by the
                // PHP backend (camelCase by default, or whatever `[crates.php] serde_rename_all`
                // overrides). The core IR's `serde_rename_all` may be None, but that has nothing
                // to do with what the binding deserializer expects.
                let config_rename_all = Some(php_lang_rename_all);
                let config_type = options_type.unwrap_or_else(|| {
                    panic!(
                        "e2e fixture {fixture_id}: handle arg `{}` requires an `options_type` on the call config (or per-language override). Set `[e2e.call] options_type = \"...\"` to the PHP class name of the handle's config struct.",
                        arg.name
                    )
                });
                setup_lines.push(format!(
                    "${name}_config = {config_type}::from_json(json_encode({}));",
                    json_to_php_camel_keys_with_types(
                        &filtered_config,
                        Some(config_type),
                        config_rename_all,
                        type_defs
                    )
                ));
                setup_lines.push(format!(
                    "${} = {class_name}::{constructor_name}(${name}_config);",
                    arg.name,
                ));
            }
            // For streaming owner_type adapters the handle is the instance-method
            // receiver, not a positional argument — emit its construction but omit
            // it from the call's argument list.
            if owner_handle_is_receiver {
                continue;
            }
            parts.push(format!("${}", arg.name));
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let emission = crate::e2e::codegen::emit_test_backend("php", trait_bridge, &methods, fixture);
                    // Split multi-line setup_block into individual lines so the
                    // Jinja template can indent each line uniformly with `        {{ line }}`.
                    for line in emission.setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(emission.arg_expr);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("php");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
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
                // PHP facades always type config as a required positional parameter.
                // Use options_type if available; otherwise infer from arg name.
                let type_name = if let Some(opt_type) = options_type {
                    opt_type.to_string()
                } else if arg.name == "config" {
                    "ExtractionConfig".to_string()
                } else {
                    format!("{}Config", arg.name.to_upper_camel_case())
                };
                // Fully qualify the type name to the binding namespace to avoid ambiguity with test namespace
                parts.push(format!("\\{namespace}\\{type_name}::from_json('{{}}')"));
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
                        if v.is_array() {
                            if elem_type == "BatchBytesItem" || elem_type == "BatchFileItem" {
                                parts.push(emit_php_batch_item_array(v, elem_type));
                                continue;
                            }
                            // When element_type is a scalar/primitive and value is an array,
                            // pass it directly as a PHP array (e.g. ["python"]) rather than
                            // wrapping in a typed config constructor.
                            if is_php_reserved_type(elem_type) {
                                parts.push(json_to_php(v));
                                continue;
                            }
                            // Tagged-enum / has_default array (e.g., Vec<PageAction>): wrap each element
                            // in `{ElemType}::from_json('{...}')`. PHP's #[php_class] FromZval only
                            // accepts class instances; raw assoc arrays produce "Failed to convert
                            // array element to {Type}". Every alef-emitted has_default/has_serde
                            // struct gets a `from_json` static method, so this is the portable path.
                            if let Some(arr) = v.as_array() {
                                let items: Vec<String> = arr
                                    .iter()
                                    .filter_map(|item| {
                                        item.as_object().map(|obj| {
                                            let json_str =
                                                serde_json::to_string(&serde_json::Value::Object(obj.clone()))
                                                    .unwrap_or_else(|_| "{}".to_string());
                                            let php_literal = json_str.replace('\\', "\\\\").replace('\'', "\\'");
                                            format!("{}::from_json('{}')", elem_type, php_literal)
                                        })
                                    })
                                    .collect();
                                parts.push(format!("[{}]", items.join(", ")));
                                continue;
                            }
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

                            // The PHP binding deserializes into the binding struct, which is
                            // always emitted with the language-effective rename strategy
                            // (camelCase by default). The core IR's serde_rename_all is not
                            // what serde_json::from_str::<Self> reads.
                            let rename_all = Some(php_lang_rename_all);
                            parts.push(format!(
                                "json_encode({})",
                                json_to_php_camel_keys_with_types(&filtered_v, options_type, rename_all, type_defs)
                            ));
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
                                        setup_lines.push(format!(
                                            "{arg_var} = \\{namespace}\\{type_name}::from_json('{{}}');"
                                        ));
                                        parts.push(arg_var);
                                        continue;
                                    }
                                }

                                let arg_var = format!("${}", arg.name);
                                // The PHP `from_json` deserializes into the binding struct, which is
                                // emitted by the PHP backend with `#[serde(rename_all = "...")]` set
                                // to the language's effective rename strategy (camelCase by default
                                // for PHP, or whatever `[crates.php] serde_rename_all` overrides).
                                // The Rust core struct's serde_rename_all is irrelevant here — what
                                // matters is the BINDING struct, since `from_json` calls
                                // `serde_json::from_str::<Self>` against the binding type.
                                let type_rename_all = Some(php_lang_rename_all);
                                setup_lines.push(format!(
                                    "{arg_var} = \\{namespace}\\{type_name}::from_json(json_encode({}));",
                                    json_to_php_camel_keys_with_types(
                                        &filtered_v,
                                        Some(type_name),
                                        type_rename_all,
                                        type_defs
                                    )
                                ));
                                parts.push(arg_var);
                                continue;
                            }
                            // Fallback: builder pattern when no options_type is configured.
                            // This path is kept for backwards compatibility with projects
                            // that use a builder-style API without explicit options_type.
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

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    fields_array_bindings: &std::collections::BTreeMap<String, (String, String)>,
    is_streaming: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->content), true)"
                );
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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
                out.push_str(&crate::e2e::template_env::render(
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

    // Skip enum variant accessors (metadata.format.excel etc.) — PHP bindings
    // serialize FormatMetadata to JSON, so variants are unavailable in PHP.
    if let Some(f) = &assertion.field {
        if f.contains("metadata.format.") && f.matches('.').count() >= 2 {
            out.push_str(&crate::e2e::template_env::render(
                "php/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `$chunks` collected-list variable.
    // Only treat a field as streaming if the call is actually streaming.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "php", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!(
                                "        $this->assertGreaterThanOrEqual({n}, count({expr}), 'expected >= {n} chunks');\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertCount({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                            format!("        $this->assertEquals('{escaped}', {expr});\n")
                        } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertEquals({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => format!("        $this->assertNotEmpty({expr});\n"),
                    "is_empty" => format!("        $this->assertEmpty({expr});\n"),
                    "is_true" => format!("        $this->assertTrue({expr});\n"),
                    "is_false" => format!("        $this->assertFalse({expr});\n"),
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertGreaterThan({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertGreaterThanOrEqual({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                            format!("        $this->assertStringContainsString('{escaped}', {expr});\n")
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
                out.push_str(&crate::e2e::template_env::render(
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
        // When result_is_simple, the result is a scalar (bytes/string/etc.) — any
        // field access on it would fail. Treat all assertions as referring to the
        // result itself.
        _ if result_is_simple => format!("${result_var}"),
        Some(f) if !f.is_empty() => {
            // Check if this field_array field has been bound to a variable
            if let Some((var_name, _)) = fields_array_bindings.get(f) {
                format!("${}", var_name)
            } else {
                let accessor = field_resolver.accessor(f, "php", &format!("${result_var}"));
                // For optional fields, wrap with ?? null to handle null-safe access
                if field_resolver.is_optional(f) {
                    format!("({accessor} ?? null)")
                } else {
                    accessor
                }
            }
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
    // values_php is consumed by `contains`, `contains_all`, and `not_contains` loops.
    // Fall back to wrapping the singular `value` so single-entry fixtures still emit one
    // assertion call per value instead of an empty loop.
    let values_php: Vec<String> = assertion
        .values
        .as_ref()
        .map(|vals| vals.iter().map(json_to_php).collect::<Vec<_>>())
        .or_else(|| assertion.value.as_ref().map(|v| vec![json_to_php(v)]))
        .unwrap_or_default();
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

    let rendered = crate::e2e::template_env::render(
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

/// Get the field type name for a given struct and field name.
/// Returns the string name of the field's type if it's a Named type, otherwise None.
fn get_field_type_name(struct_name: &str, field_name: &str, type_defs: &[crate::core::ir::TypeDef]) -> Option<String> {
    type_defs
        .iter()
        .find(|td| td.name == struct_name)
        .and_then(|td| td.fields.iter().find(|f| f.name == field_name))
        .and_then(|field| match &field.ty {
            TypeRef::Named(name) => Some(name.clone()),
            TypeRef::Optional(inner) => match &**inner {
                TypeRef::Named(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        })
}

/// Like `json_to_php` but optionally converts object keys to lowerCamelCase.
/// When `serde_rename_all` is Some("camelCase"), recursively converts all object keys
/// from snake_case to camelCase. Otherwise, passes keys through unchanged.
///
/// Uses IR type information to determine the correct serde_rename_all setting for
/// nested structs — each nested object's keys are transformed based on whether that
/// specific struct type has `#[serde(rename_all = "camelCase")]`, not inherited from
/// the parent.
///
/// Used when generating PHP option arrays passed to `from_json()` — PHP binding
/// structs respect the serde attributes of the underlying Rust core types, so we only
/// apply camelCase transformation when the target type explicitly declares it.
fn json_to_php_camel_keys_with_types(
    value: &serde_json::Value,
    current_type_name: Option<&str>,
    serde_rename_all: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let final_key = if serde_rename_all == Some("camelCase") {
                        k.to_lower_camel_case()
                    } else {
                        k.to_string()
                    };
                    // When recursing into a nested object, propagate the parent's
                    // serde_rename_all. For PHP this matters because all binding structs are
                    // emitted with the same `#[serde(rename_all = "...")]` setting (driven by
                    // the language-effective rename strategy), so nested objects use the same
                    // strategy as the parent. The Rust core type's serde_rename_all on the
                    // nested field's type is irrelevant — the binding deserializer reads the
                    // binding struct's attributes.
                    let nested_type_name = current_type_name.and_then(|tn| get_field_type_name(tn, k, type_defs));
                    format!(
                        "\"{}\" => {}",
                        escape_php(&final_key),
                        json_to_php_camel_keys_with_types(v, nested_type_name.as_deref(), serde_rename_all, type_defs)
                    )
                })
                .collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|item| json_to_php_camel_keys_with_types(item, current_type_name, serde_rename_all, type_defs))
                .collect();
            format!("[{}]", items.join(", "))
        }
        _ => json_to_php(value),
    }
}

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a PHP visitor object and add setup lines. The visitor is assigned to $visitor variable.
fn build_php_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::e2e::fixture::VisitorSpec) {
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

    let (action_type, action_value, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), "dict"),
        CallbackAction::Custom { output } => ("custom", escape_php(output), "dict"),
        CallbackAction::CustomTemplate { template, return_form } => {
            let form = match return_form {
                TemplateReturnForm::Dict => "dict",
                TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", escape_php(template), form)
        }
    };

    let rendered = crate::e2e::template_env::render(
        "php/visitor_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params,
            action_type => action_type,
            action_value => action_value,
            return_form => return_form,
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

/// Extract the canonical backend name from fixture input JSON.
///
/// Mirrors the lookup strategy used by the Python and Rust e2e emitters.
/// Searches `input.name`, then any nested object's `name` field, then falls
/// back to `fixture_id`.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Emit a PHP test backend stub.
///
/// PHP is duck-typed: define an anonymous class inside the test method body.
/// Each method returns a sensible PHP default. The Plugin super-trait `name`
/// method returns the backend name extracted from `fixture.input`.
///
/// The returned `setup_block` contains the inline class declaration.
/// The `arg_expr` is `$stub`.
/// Callers emit `Kreuzberg::<RegisterFn>($stub)`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::sanitize_ident;

    let defaults = language_defaults("php");
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);

    // Build setup_block lines without leading indentation: the Jinja template
    // prefixes each line with 8 spaces (two method-body indent levels in PHPUnit).
    let mut setup = String::new();
    // PHP anonymous class must implement the interface explicitly.
    let interface_name = trait_bridge.trait_name.to_upper_camel_case();
    let _ = writeln!(setup, "$stub = new class implements {interface_name} {{");

    // Plugin super-trait: emit `name()` returning the backend name string.
    if trait_bridge.super_trait.is_some() {
        let escaped_name = escape_php(&backend_name);
        let _ = writeln!(
            setup,
            "    public function name(): string {{ return '{escaped_name}'; }}"
        );
    }

    // Emit stubs for all required methods (skip those with default implementations).
    for method in methods.iter().filter(|m| !m.has_default_impl) {
        let php_name = method.name.to_lower_camel_case();
        // Named types are not defined in the PHP binding scope.  The PHP bridge
        // deserialises the return value via json_decode, so return a JSON-safe
        // empty-object string instead of attempting a constructor call.
        let default_val = match &method.return_type {
            TypeRef::Named(_) => "'{}'".to_string(),
            other => defaults.emit_default(other),
        };
        // Parameter list: positional only (PHP is duck-typed; we omit type hints for simplicity).
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("${}", sanitize_ident(&p.name)))
            .collect();
        let param_str = params.join(", ");
        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(setup, "    public function {php_name}({param_str}): void {{}}");
        } else {
            let _ = writeln!(
                setup,
                "    public function {php_name}({param_str}): mixed {{ return {default_val}; }}"
            );
        }
    }

    let _ = writeln!(setup, "}};");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr: "$stub".to_string(),
        type_imports: Vec::new(),
    }
}

#[cfg(test)]
mod trait_bridge_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;

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

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
        }
    }

    fn make_method(name: &str, params: Vec<(&str, TypeRef)>, ret: TypeRef, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: params.into_iter().map(|(n, ty)| make_param(n, ty)).collect(),
            return_type: ret,
            is_async,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Genericity test: a synthetic TestTrait with one sync method and Plugin super-trait
    /// must not reference any kreuzberg-domain names in setup_block or arg_expr.
    #[test]
    fn test_backend_emission_is_generic() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("SomeSuperTrait".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..TraitBridgeConfig::default()
        };

        let do_thing = make_method(
            "do_thing",
            vec![("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
            TypeRef::String,
            false,
        );

        let fixture = make_fixture("my_test_fixture");
        let methods = vec![&do_thing];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // setup_block must not reference any kreuzberg-domain trait or method names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode domain trait names, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("process_image"),
            "setup_block must not hardcode domain method names, got:\n{}",
            emission.setup_block
        );
        // Must emit the method name from MethodDef (PHP camelCase).
        assert!(
            emission.setup_block.contains("doThing"),
            "setup_block must contain the PHP camelCase method name 'doThing', got:\n{}",
            emission.setup_block
        );
        // Must emit Plugin name method when super_trait is set.
        assert!(
            emission.setup_block.contains("name"),
            "setup_block must emit 'name' for super_trait, got:\n{}",
            emission.setup_block
        );
        // arg_expr is the anonymous class variable.
        assert_eq!(
            emission.arg_expr, "$stub",
            "arg_expr must be '$stub', got: {}",
            emission.arg_expr
        );
    }

    /// Named return types must emit `'{}'` (JSON-safe empty-object string), not
    /// a constructor call that would reference an undefined class.
    #[test]
    fn test_backend_named_return_emits_json_string() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes), ("mime_type", TypeRef::String)],
            TypeRef::Named("InternalDocument".to_string()),
            false,
        );

        let fixture = make_fixture("register_document_extractor_trait_bridge");
        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("'{}'"),
            "Named return type must emit '{{}}' not a constructor call, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("new InternalDocument"),
            "setup_block must not reference undefined type InternalDocument, got:\n{}",
            emission.setup_block
        );
    }

    /// Backend name is extracted from fixture.input, not fixture.id.
    #[test]
    fn test_backend_name_from_input() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes)],
            TypeRef::Named("InternalDocument".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("test-extractor"),
            "setup_block must use input-derived name 'test-extractor', got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission
                .setup_block
                .contains("register_document_extractor_trait_bridge"),
            "setup_block must not use fixture id as name, got:\n{}",
            emission.setup_block
        );
    }

    /// Snapshot: verify exact setup_block shape for a DocumentExtractor-like bridge.
    #[test]
    fn test_backend_snapshot() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![
                ("content", TypeRef::Bytes),
                ("mime_type", TypeRef::String),
                ("config", TypeRef::Named("ExtractionConfig".to_string())),
            ],
            TypeRef::Named("InternalDocument".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        let expected_setup = concat!(
            "$stub = new class implements DocumentExtractor {\n",
            "    public function name(): string { return 'test-extractor'; }\n",
            "    public function extractBytes($content, $mime_type, $config): mixed { return '{}'; }\n",
            "};\n",
        );
        assert_eq!(emission.setup_block, expected_setup, "setup_block snapshot mismatch");
        assert_eq!(emission.arg_expr, "$stub");
    }
}

#[cfg(test)]
mod composer_json_tests {
    use super::{render_composer_json, render_install_sh};
    use crate::e2e::config::DependencyMode;

    #[test]
    fn registry_composer_json_uses_ext_platform_req() {
        let content = render_composer_json(
            "kreuzberg/e2e-php",
            "LiterLlm\\\\E2e\\\\",
            "liter_llm",
            "kreuzberg/liter-llm",
            "../../packages/php",
            "1.4.0-rc.32",
            DependencyMode::Registry,
        );
        // Must declare the ext-<name> platform require.
        assert!(
            content.contains(r#""ext-liter_llm": "*""#),
            "registry composer.json must require ext-liter_llm: *, got:\n{content}"
        );
        // Must declare the php platform require.
        assert!(
            content.contains(r#""php": ">=8.2""#),
            "registry composer.json must require php >=8.2, got:\n{content}"
        );
        // Must NOT contain a direct package require (composer can't resolve it
        // before PIE has installed the .so).
        assert!(
            !content.contains("kreuzberg/liter-llm"),
            "registry composer.json must not contain a direct package require, got:\n{content}"
        );
        // Must NOT carry minimum-stability / prefer-stable (not load-bearing with
        // platform reqs only).
        assert!(
            !content.contains("minimum-stability"),
            "registry composer.json must not contain minimum-stability, got:\n{content}"
        );
        assert!(
            !content.contains("prefer-stable"),
            "registry composer.json must not contain prefer-stable, got:\n{content}"
        );
        // Must keep require-dev (phpunit + guzzle).
        assert!(
            content.contains("phpunit/phpunit"),
            "registry composer.json must keep phpunit in require-dev, got:\n{content}"
        );
        assert!(
            content.contains("guzzlehttp/guzzle"),
            "registry composer.json must keep guzzle in require-dev, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_contains_pie_install() {
        let content = render_install_sh("kreuzberg/liter-llm", "liter_llm", "1.4.0-rc.32");
        // The script uses $PIE as the resolved pie binary path.
        assert!(
            content.contains("\"$PIE\" install"),
            "install.sh must invoke pie via $PIE install, got:\n{content}"
        );
        assert!(
            content.contains("kreuzberg/liter-llm"),
            "install.sh must reference the package name, got:\n{content}"
        );
        assert!(
            content.starts_with("#!/usr/bin/env bash"),
            "install.sh must start with bash shebang, got:\n{content}"
        );
        // Version is baked in so callers can run `bash install.sh` with no args.
        assert!(
            content.contains(r#"VERSION="${1:-1.4.0-rc.32}""#),
            "install.sh must default VERSION to the alef-pinned version, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_strips_version_constraints() {
        // Test constraint operators are stripped from version strings.
        let tests = vec![
            (">=3.5.1", "3.5.1"),
            ("^1.2.3", "1.2.3"),
            ("~2.0.0", "2.0.0"),
            (">1.0", "1.0"),
            ("<2.0", "2.0"),
            ("1.4.0-rc.32", "1.4.0-rc.32"), // Already clean
        ];
        for (input, expected) in tests {
            let content = render_install_sh("test/pkg", "ext", input);
            assert!(
                content.contains(&format!(r#"VERSION="${{1:-{expected}}}""#)),
                "install.sh must strip constraint from '{}' to '{}', got:\n{}",
                input,
                expected,
                content
            );
        }
    }

    #[test]
    fn registry_install_sh_downloads_pie_phar() {
        let content = render_install_sh("test/pkg", "ext", "1.0.0");
        // Ensure the script downloads PIE as a PHAR, not via composer require.
        assert!(
            content.contains("https://github.com/php/pie/releases/latest/download/pie.phar"),
            "install.sh must download PIE PHAR from GitHub, got:\n{content}"
        );
        assert!(
            !content.contains("composer global require php/pie"),
            "install.sh must not use composer to install PIE, got:\n{content}"
        );
    }
}
