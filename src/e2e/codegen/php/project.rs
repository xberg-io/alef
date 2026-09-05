//! PHP e2e project-level file and bootstrap renderers.
//!
//! These helpers were previously defined in `php.rs` and are preserved here for
//! modularization.

use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

/// Build the `"autoload"` PSR-4 section that maps the binding's PHP userland
/// namespace to the generated class directory.
///
/// The consumer's userland classes (layered over the native ext-php-rs
/// extension) are NOT registered by the native extension, so PHPUnit cannot
/// resolve them without this mapping. Both dependency modes need it: `e2e/php`
/// (Local) and `test_apps/php` (Registry) sit at the same depth relative to the
/// repository root, so the same relative `autoload_target` is correct for both.
///
/// `autoload_target` is the class directory itself, already relative to the
/// generated project. This function must not append a `src/` segment of its
/// own: the layout that directory lives in is decided by
/// [`php_psr4_target`](crate::backends::php::layout::php_psr4_target), and a
/// second opinion here is what previously pointed the e2e project at a
/// `.../src/` subdirectory of the real class tree that no alef stage writes. ~keep
///
/// `pkg_namespace` is the resolved `[crates.php] namespace` and is used
/// verbatim as the PSR-4 prefix. It must never be re-derived from the composer
/// package name: word-splitting a namespace like `WidgetToolkit` into
/// `Widget\Toolkit` yields a prefix that matches no declared namespace, so
/// Composer silently autoloads nothing. Namespaces that already contain `\`
/// (e.g. `Acme\Widget`) are preserved as written; only JSON escaping is
/// applied.
fn php_autoload_section(pkg_namespace: &str, autoload_target: &str) -> String {
    format!(
        r#"
  "autoload": {{
    "psr-4": {{
      "{}\\": "{}"
    }}
  }},"#,
        pkg_namespace.replace('\\', "\\\\"),
        autoload_target
    )
}

pub(super) fn render_composer_json(
    e2e_pkg_name: &str,
    e2e_autoload_ns: &str,
    _extension_name: &str,
    pkg_namespace: &str,
    autoload_target: &str,
    _pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let (require_section, autoload_section) = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Registry-mode test_apps run `install.sh` before `composer install`.
            // That script boots the PIE extension and installs the extension binary
            // into the system's extension_dir. Once PIE has installed the extension,
            // it will be loaded when PHP starts (via default php.ini or explicit
            // `-dextension=`).
            //
            // The test_app's composer.json does NOT declare `ext-<name>` as a
            // requirement because:
            // 1. The extension is installed via PIE, not Composer (Composer can't
            //    install system binaries).
            // 2. Declaring `ext-<name>: "*"` in composer.json causes Composer's
            //    platform resolver to check `php -m` for the extension. If the
            //    extension hasn't been loaded into the running PHP process yet (which
            //    it won't be until a fresh PHP invocation with the extension loaded),
            //    Composer fails with:
            //    "Root composer.json requires PHP extension ext-<name> * but it is
            //    missing".
            // 3. The extension is guaranteed to be loaded before tests run
            //    (install.sh ensures this).
            //
            // `php: ">=8.2"` is sufficient — Composer verifies the PHP version at
            // runtime (always satisfied on CI runners) and development dependencies
            // (phpunit, guzzle) are the only packages Composer needs to manage.
            let require = format!(
                r#"  "require": {{
    "php": ">=8.2"
  }},
  "require-dev": {{
    "phpunit/phpunit": "{phpunit}",
    "guzzlehttp/guzzle": "{guzzle}"
  }},"#,
                phpunit = tv::packagist::PHPUNIT,
                guzzle = tv::packagist::GUZZLE,
            );
            // The userland PHP classes (layered over the PIE-installed native
            // extension) are autoloaded from the local package source, since the
            // registry `require` deliberately omits the package itself.
            (require, php_autoload_section(pkg_namespace, autoload_target))
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
            (require, php_autoload_section(pkg_namespace, autoload_target))
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

/// Render the `install.sh` script placed next to `composer.json` in registry mode.
///
/// The script downloads the exact `php/pie` release alef pins, verifies its SHA-256 before
/// executing it, runs `pie install <pkg>:<version>`, and verifies the extension binary
/// loads. It does this unconditionally: an already-installed `pie` on `PATH` is never
/// reused, however new it claims to be. It used to be reused whenever it reported >= 1.3.7,
/// which meant the pin and the digest only ever applied on a machine that happened to have
/// no PIE -- everywhere else an arbitrary unverified binary ran instead, and two machines
/// executed two different PIEs. Do not reintroduce a version-sniffing fast path. ~keep
///
/// The package version is baked in at generate time; callers run `bash install.sh` with no
/// arguments. The default `alef test-apps run` command for PHP invokes this script before
/// `composer install`.
/// Strip leading composer-style version constraints (^, >=, ~, etc.) from a version string.
/// Accepts "1.2.3", ">=1.2.3", "^1.2.3", "~1.2", or any constraint and returns the base version.
pub(super) fn strip_version_constraint(version: &str) -> &str {
    version.trim_start_matches(['^', '~', '>', '<', '='])
}

pub(super) fn render_install_sh(pkg_name: &str, extension_name: &str, pkg_version: &str) -> String {
    let clean_version = strip_version_constraint(pkg_version);
    let quote = crate::core::config::shell::quote_word;
    let pkg_name = quote(pkg_name);
    let extension_name = quote(extension_name);
    let clean_version = quote(clean_version);
    let pie_version = crate::core::template_versions::github_release::PIE_VERSION;
    let pie_sha256 = crate::core::template_versions::github_release::PIE_PHAR_SHA256;
    // `generated_header: false` on this GeneratedFile (see php.rs) means ownership tracking
    // relies entirely on `hash::content_has_alef_marker` recognizing text embedded in the
    // rendered content itself -- so the marker line must come from the shared authority
    // (`hash::header`) rather than a hand-spelled "alef-generated" line that guard doesn't
    // recognize, which would strand the file unowned forever. ~keep
    let header = hash::header(CommentStyle::Hash);
    format!(
        r#"#!/usr/bin/env bash
{header}# Installs the configured extension via PIE before `composer install` runs.
# Requires `php` on PATH; always downloads and checksum-verifies its own PIE.
# Version is alef-injected at generate time so the script is self-contained.
set -euo pipefail

# Version override: pass as $1 to test an arbitrary tag; defaults to the
# alef-pinned version from `[crates.e2e.registry.packages.php].version`.
PKG_NAME={pkg_name}
EXTENSION_NAME={extension_name}
PINNED_VERSION={clean_version}
VERSION="${{1:-$PINNED_VERSION}}"

# PIE itself is pinned to an exact release and its PHAR is checksum-verified before it is
# ever executed. This used to fetch the floating newest-release asset, so whatever upstream
# published at run time was executed unverified, and no past run could be reproduced.
# PIE_VERSION and PIE_PHAR_SHA256 are one pair in alef's template_versions registry and must
# always be bumped together.
PIE_VERSION={pie_version}
PIE_PHAR_SHA256={pie_sha256}

# The downloaded, digest-verified PHAR is the ONLY PIE this script ever runs. There is
# deliberately no "an installed `pie` already looks new enough, reuse it" branch: that
# branch made the pin and the digest apply only on machines that happened to have no PIE,
# so an arbitrary preinstalled binary ran unverified everywhere else and two machines
# executed two different PIEs. PATH is never consulted to choose the interpreter.
#
# The PHAR lands in an alef-owned, version-scoped cache directory rather than
# `~/.local/bin`, so an unconditional install cannot clobber a PIE the developer installed
# themselves. That directory is prepended to PATH so anything downstream that shells `pie`
# by name also gets the verified PHAR and not a preinstalled one further down PATH.
pie_dir="${{HOME}}/.cache/alef/pie/$PIE_VERSION"
mkdir -p "$pie_dir"
pie_tmp="$(mktemp "${{TMPDIR:-/tmp}}/pie.phar.XXXXXX")"
trap 'rm -f "$pie_tmp"' EXIT
# stderr is NOT discarded: the previous `2>/dev/null` hid the actual reason (404, TLS,
# proxy) behind a generic message every time this failed in CI.
curl -fL --output "$pie_tmp" "https://github.com/php/pie/releases/download/$PIE_VERSION/pie.phar" || {{
  echo "::error::Failed to download PIE $PIE_VERSION from GitHub; ensure network access." >&2
  exit 1
}}
if command -v sha256sum >/dev/null 2>&1; then
  pie_actual_sha256="$(sha256sum "$pie_tmp" | awk '{{print $1}}')"
elif command -v shasum >/dev/null 2>&1; then
  pie_actual_sha256="$(shasum -a 256 "$pie_tmp" | awk '{{print $1}}')"
else
  # Hard failure, not a skip: a verification step that silently passes when its tool is
  # missing verifies nothing, and every supported platform ships one of these two.
  echo "::error::No sha256 tool (sha256sum or shasum) found; refusing to run an unverified PIE PHAR." >&2
  exit 1
fi
if [[ "$pie_actual_sha256" != "$PIE_PHAR_SHA256" ]]; then
  echo "::error::PIE $PIE_VERSION checksum mismatch: expected $PIE_PHAR_SHA256, got $pie_actual_sha256" >&2
  exit 1
fi
install -m 0755 "$pie_tmp" "$pie_dir/pie"
PIE="$pie_dir/pie"
export PATH="$pie_dir:$PATH"

# Install the extension binary into the running PHP's extension dir.
# Always run PIE — an existence-only skip leaves a stale .so from a prior rc
# (different ABI / missing symbols) in $EXT_DIR, which then fails the verification
# step below. PIE itself is idempotent: re-installing overwrites the existing
# binary cleanly. The php.ini-append guard below prevents duplicate `extension=`
# lines so the verification step doesn't trip on "Module already loaded".
EXT_DIR="$(php -r 'echo ini_get("extension_dir");')"
# PIE's `install` has no `--version` option (it parses `--version`/`-V` as
# "print PIE's own version" and exits without installing). The target version is
# part of the package coordinate: `vendor/package:constraint`.
"$PIE" install "$PKG_NAME:$VERSION" --skip-enable-extension

# Verify the .so/.dylib/.dll exists after install (or was already present).
test -f "$EXT_DIR/$EXTENSION_NAME.so" || test -f "$EXT_DIR/$EXTENSION_NAME.dylib" || test -f "$EXT_DIR/$EXTENSION_NAME.dll"

# Enable the extension in php.ini (PIE with --skip-enable-extension doesn't do this automatically).
# Find the loaded php.ini, check if already enabled, and append if missing.
PHP_INI="$(php --ini 2>&1 | grep -m1 'Loaded Configuration File:' | awk '{{print $NF}}')"
if [[ -z "$PHP_INI" ]]; then
  echo "::warning::Could not locate php.ini; extension may not be auto-loaded by default" >&2
else
  if [[ ! -f "$PHP_INI" ]]; then
    echo "::warning::php.ini at $PHP_INI not found; extension may not be auto-loaded by default" >&2
  else
    # Guard against duplicate: check if extension line already exists (uncommented).
    if ! grep -Fqx "extension=$EXTENSION_NAME" "$PHP_INI"; then
      printf 'extension=%s\n' "$EXTENSION_NAME" >> "$PHP_INI"
    fi
  fi
fi

# Export the installed extension path for downstream test runners (composer test).
# The test app's run_tests.php checks for PIE_INSTALLED_EXTENSION_PATH and loads the extension via `-d`.
export PIE_INSTALLED_EXTENSION_PATH="$EXT_DIR/$EXTENSION_NAME.so"
if [[ "$OSTYPE" == "darwin"* ]]; then
  export PIE_INSTALLED_EXTENSION_PATH="$EXT_DIR/$EXTENSION_NAME.dylib"
fi

# Verify the extension loads. Use `extension_loaded()` via `php -r` instead of
# parsing `php -m` output: `php -m` is fragile when an extension is enabled via
# both php.ini *and* a conf.d drop-in (e.g. when a prior PIE install left a
# conf.d entry behind), because PHP prints "Module ... is already loaded" to
# stderr and the test harness 2>&1 capture treats it as fatal. `extension_loaded`
# checks runtime state directly and is unaffected by load source or stderr noise.
if ALEF_EXTENSION_NAME="$EXTENSION_NAME" php -r 'exit(extension_loaded(getenv("ALEF_EXTENSION_NAME")) ? 0 : 1);' 2>/dev/null; then
  echo "$EXTENSION_NAME extension loaded via php.ini"
elif ALEF_EXTENSION_NAME="$EXTENSION_NAME" php -d "extension=$EXTENSION_NAME" -r 'exit(extension_loaded(getenv("ALEF_EXTENSION_NAME")) ? 0 : 1);' 2>/dev/null; then
  echo "$EXTENSION_NAME extension loaded via -d flag"
else
  echo "::error::$EXTENSION_NAME extension failed to load after PIE install" >&2
  exit 1
fi
echo "$EXTENSION_NAME extension installed and loaded"
"#
    )
}

pub(super) fn render_phpunit_xml() -> String {
    crate::e2e::template_env::render("php/phpunit.xml.jinja", minijinja::context! {})
}

/// Render the app harness script for server-pattern HTTP fixtures.
///
/// The harness script spawns the SUT app and registers handlers per fixture,
/// returning canned expected responses. It's driven by bootstrap.php's subprocess
/// launcher.
///
/// # Note
///
/// This function is retained for reference but no longer called from alef.
/// A consumer extension now owns `app_harness.php` emission.
#[allow(dead_code)]
pub(super) fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup], pkg_path: &str) -> String {
    use serde_json::json;

    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            // We only need the http field, handler, request, and expected_response.
            let http_data = fixture.http.as_ref().unwrap();
            let fixture_json = json!({
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
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json = serde_json::to_string(&fixtures_map).unwrap_or_default();

    let imports = &e2e_config.harness.imports;
    let app_class = e2e_config.harness.app_class_for_lang("php");
    // PHP wraps via ext-php-rs which historically emits snake_case method names
    // from the IR. `register_method_idiomatic` keeps snake_case for PHP so the
    // call site matches what the service-API codegen emits.
    let register_route_method = e2e_config
        .harness
        .register_method_idiomatic("php")
        .unwrap_or_else(|| "route".to_string());
    let body_schema_setter = &e2e_config.harness.body_schema_setter;
    let method_enum = &e2e_config.harness.method_enum;
    let run_method = e2e_config.harness.run_method_for_lang("php");
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;

    let header = hash::header(CommentStyle::DoubleSlash);

    // Derive route_builder_import from imports[0] → PHP namespace.
    // E.g. imports[0] = "my_pkg" → namespace MyPkg\Php
    let route_builder_import = if !imports.is_empty() {
        let module_name = &imports[0];
        // Normalize module name to PHP namespace (my_pkg → MyPkg, sample_core → SampleCore)
        module_name
            .split('_')
            .map(|p| {
                let mut chars = p.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join("\\")
            + "\\Php"
    } else {
        "App\\Php".to_string()
    };
    let method_enum_import = route_builder_import.clone();

    let ctx = minijinja::context! {
        header => header,
        imports => imports,
        app_class => app_class.as_deref().unwrap_or("App"),
        route_builder_import => route_builder_import,
        route_builder_class => "RouteBuilder",
        register_route_method => register_route_method.as_str(),
        route_builder_schema_setter => body_schema_setter.as_deref().unwrap_or("request_schema_json"),
        method_enum_import => method_enum_import,
        method_enum_class => method_enum.as_deref().unwrap_or("Method"),
        run_method => run_method.as_deref().unwrap_or("run"),
        response_body_field => e2e_config.harness.response_body_field.as_str(),
        host => host,
        port => port,
        pkg_path => pkg_path,
        fixtures_json => fixtures_json,
    };

    crate::e2e::template_env::render("php/app_harness.php.jinja", ctx)
}

/// Emit PHP code that sets every `[e2e.env]` entry into the environment
/// using the setdefault pattern (check getenv, then update putenv + $_ENV + $_SERVER).
/// Returns empty when no env vars are configured.
///
/// `env` is a `BTreeMap`, so this already iterates in key order -- no separate sort needed. ~keep
fn render_env_setup_block(e2e_config: &E2eConfig) -> String {
    if e2e_config.env.is_empty() {
        return String::new();
    }
    let lines = e2e_config
        .env
        .iter()
        .map(|(k, v)| {
            format!(
                "if (getenv('{}') === false) {{\n    putenv('{}={}');\n    $_ENV['{}'] = '{}';\n    $_SERVER['{}'] = '{}';\n}}",
                k, k, v, k, v, k, v
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{}\n\n", lines)
}

pub(super) struct BootstrapOptions<'a> {
    pub(super) e2e_config: &'a E2eConfig,
    pub(super) pkg_path: &'a str,
    pub(super) has_mock_server_fixtures: bool,
    pub(super) has_file_fixtures: bool,
    pub(super) test_documents_path: &'a str,
    pub(super) uses_server_harness: bool,
    pub(super) harness_host: &'a str,
    pub(super) harness_port: u16,
}

pub(super) fn render_bootstrap(options: BootstrapOptions<'_>) -> String {
    let BootstrapOptions {
        e2e_config,
        pkg_path,
        has_mock_server_fixtures,
        has_file_fixtures,
        test_documents_path,
        uses_server_harness,
        harness_host,
        harness_port,
    } = options;
    let header = hash::header(CommentStyle::DoubleSlash);
    let env_setup = render_env_setup_block(e2e_config);
    crate::e2e::template_env::render(
        "php/bootstrap.php.jinja",
        minijinja::context! {
            header => header,
            env_setup => env_setup,
            pkg_path => pkg_path,
            has_mock_server_fixtures => has_mock_server_fixtures,
            has_file_fixtures => has_file_fixtures,
            test_documents_path => test_documents_path,
            uses_server_harness => uses_server_harness,
            harness_host => harness_host,
            harness_port => harness_port,
        },
    )
}

/// Render `run_tests.php`, the generated PHP e2e entrypoint.
///
/// The extension load order is the crux of this file: an ambient, previously
/// installed copy of the extension (e.g. published via PIE by an earlier
/// release) registers itself during ordinary php.ini processing, which
/// happens *before* any `-d extension=` override on the command line takes
/// effect. When both declare the same extension name, PHP keeps whichever
/// loaded first -- the ambient one -- and only emits a "Module ... is already
/// loaded" warning for the one this harness asked for, so the freshly built
/// extension at `$extPath` is silently discarded in favor of stale code with
/// no non-zero exit anywhere. `-n` (drop every ini file) is not a fix on its
/// own: PHPUnit needs several extensions (dom, json, libxml, mbstring,
/// tokenizer, xml, xmlwriter, ...) that are shared modules on some platforms
/// and statically compiled on others, so hardcoding which ones to
/// re-declare under `-n` breaks whichever platform guessed wrong.
///
/// The fix built into this template is to construct an "isolated" php.ini at
/// run time: read the ambient php.ini plus every ini file PHP would itself
/// scan from its conf.d directory, drop only the `extension=`/`zend_extension=`
/// line(s) that name *this* extension, and load PHP with `-n -c <that file>
/// -d extension=<built path>`. Every other ambient setting PHPUnit or the
/// extension needs survives untouched; only the one directive that would
/// collide with the fresh build is removed. On a machine with no ambient
/// install at all, no line matches the filter, the isolated ini is a faithful
/// copy of the ambient one, and behavior is unchanged. ~keep
///
/// A preflight check then runs an isolated, one-shot PHP invocation that
/// loads only $extPath and asserts both its identity (`extension_loaded`)
/// and its exact reported version, capturing stdout and stderr separately so
/// an "already loaded" startup warning is caught even though it does not by
/// itself produce a non-zero exit code. This check is unconditional -- it is
/// not gated behind a re-exec environment variable -- specifically because a
/// prior version of this template put an equivalent check after a branch
/// that always `passthru`'d straight into PHPUnit and `exit`'d, so the check
/// could never be reached at all. A guard that cannot run is worse than
/// having none, because it creates false confidence. ~keep
pub(super) fn render_run_tests_php(
    extension_name: &str,
    cargo_crate_name: Option<&str>,
    cargo_package_name: &str,
    pkg_version: &str,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let ext_lib_name = if let Some(crate_name) = cargo_crate_name {
        // Cargo replaces hyphens with underscores for lib names, and the crate name
        // already includes the _php suffix.
        format!("lib{}", crate_name.replace('-', "_"))
    } else {
        format!("lib{extension_name}_php")
    };
    const TEMPLATE: &str = r#"#!/usr/bin/env php
<?php
__HEADER__
declare(strict_types=1);

// Determine platform-specific extension suffix.
$extSuffix = match (PHP_OS_FAMILY) {
    'Darwin' => '.dylib',
    default => '.so',
};
$localExtPath = __DIR__ . '/../../target/release/__EXT_LIB_NAME__' . $extSuffix;
$extPath = $localExtPath;

// Check for PIE-installed extension path (set by install.sh in registry mode).
// In registry mode, the extension is installed system-wide via PIE and passed
// via the PIE_INSTALLED_EXTENSION_PATH environment variable.
$pieInstalledExtPath = getenv('PIE_INSTALLED_EXTENSION_PATH');
if ($pieInstalledExtPath && file_exists($pieInstalledExtPath)) {
    $extPath = $pieInstalledExtPath;
}

// Neither a local release build nor a PIE-installed extension was found. Fail
// loudly instead of falling through to PHPUnit: the ambient php.ini may still
// register a system-installed copy of this extension (e.g. from a previous
// release), and silently testing against it exercises stale, uncontrolled
// code instead of this checkout. ~keep
if (!file_exists($extPath)) {
    fwrite(STDERR, "error: no __EXTENSION_NAME__ PHP extension build found.\n");
    fwrite(STDERR, "  looked for a local build at: $localExtPath\n");
    $pieDisplay = $pieInstalledExtPath !== false ? $pieInstalledExtPath : '(unset)';
    fwrite(STDERR, "  looked for PIE_INSTALLED_EXTENSION_PATH at: $pieDisplay\n");
    fwrite(STDERR, "Build it locally with:\n");
    fwrite(STDERR, "  cargo build --release -p __CARGO_PACKAGE_NAME__\n");
    exit(1);
}

// Build an isolated php.ini carrying every ambient ini directive EXCEPT any
// `extension=`/`zend_extension=` line naming this extension. This runs the
// preflight check and PHPUnit under a config where only this build's own
// `-d extension=` can register the extension, instead of racing an ambient
// php.ini entry that would otherwise register it first. ~keep
function alef_build_isolated_ini(string $php, string $extensionName): string {
    $iniInfo = [];
    exec(escapeshellarg($php) . ' --ini 2>&1', $iniInfo);

    $mainIni = null;
    $scanDir = null;
    foreach ($iniInfo as $line) {
        if (preg_match('/^Loaded Configuration File:\s*(.+)$/', $line, $m)) {
            $value = trim($m[1]);
            $mainIni = ($value !== '' && $value !== '(none)') ? $value : null;
        }
        if (preg_match('/^Scan for additional \.ini files in:\s*(.+)$/', $line, $m)) {
            $value = trim($m[1]);
            $scanDir = ($value !== '' && $value !== '(none)') ? $value : null;
        }
    }

    $sourceFiles = [];
    if ($mainIni !== null && is_file($mainIni)) {
        $sourceFiles[] = $mainIni;
    }
    if ($scanDir !== null && is_dir($scanDir)) {
        $confFiles = glob($scanDir . '/*.ini') ?: [];
        sort($confFiles);
        $sourceFiles = array_merge($sourceFiles, $confFiles);
    }

    $extensionPattern = '/^\s*(?:zend_)?extension\s*=\s*(.+?)\s*$/i';
    $lines = [];
    foreach ($sourceFiles as $sourceFile) {
        $content = file_get_contents($sourceFile);
        if ($content === false) {
            continue;
        }
        foreach (preg_split('/\R/', $content) as $line) {
            if (preg_match($extensionPattern, $line, $m)) {
                $value = trim($m[1], " \t\"'");
                $base = basename($value);
                $base = preg_replace('/\.(so|dylib|dll)$/i', '', $base);
                if (strcasecmp($base, $extensionName) === 0) {
                    continue;
                }
            }
            $lines[] = $line;
        }
    }

    $isolatedIni = tempnam(sys_get_temp_dir(), 'alef-php-ini-');
    file_put_contents($isolatedIni, implode("\n", $lines) . "\n");
    return $isolatedIni;
}

// Indent every line of $text by four spaces, for nesting a captured
// subprocess stream inside a `fwrite(STDERR, ...)` diagnostic block.
function alef_indent_lines(string $text): string {
    return implode("\n    ", explode("\n", trim($text)));
}

$php = PHP_BINARY;
$isolatedIni = alef_build_isolated_ini($php, '__EXTENSION_NAME__');
register_shutdown_function(static function () use ($isolatedIni): void {
    if (is_file($isolatedIni)) {
        unlink($isolatedIni);
    }
});

$phpConfigArgs = ['-n', '-c', $isolatedIni, '-d', 'extension=' . $extPath];

// Preflight: prove the isolated invocation actually loads THIS build before
// trusting it for the real test run. Stdout and stderr are captured
// separately so an "already loaded" startup warning is caught even though it
// does not by itself affect the exit code. ~keep
$preflightScript =
    '$name = getenv("ALEF_PREFLIGHT_EXT_NAME"); ' .
    'if (!extension_loaded($name)) { fwrite(STDERR, "not-loaded"); exit(2); } ' .
    'echo phpversion($name);';
$preflightCmd = array_merge(
    [$php],
    $phpConfigArgs,
    ['-r', $preflightScript]
);
$descriptorSpec = [1 => ['pipe', 'w'], 2 => ['pipe', 'w']];
$process = proc_open(
    $preflightCmd,
    $descriptorSpec,
    $pipes,
    null,
    ['ALEF_PREFLIGHT_EXT_NAME' => '__EXTENSION_NAME__'] + $_ENV,
    ['bypass_shell' => true]
);
if ($process === false) {
    fwrite(STDERR, "error: failed to spawn PHP preflight check for the __EXTENSION_NAME__ extension.\n");
    exit(1);
}
$preflightStdout = stream_get_contents($pipes[1]);
$preflightStderr = stream_get_contents($pipes[2]);
fclose($pipes[1]);
fclose($pipes[2]);
$preflightExit = proc_close($process);

if (stripos($preflightStderr, 'already loaded') !== false) {
    fwrite(STDERR, "error: the __EXTENSION_NAME__ extension is already loaded from an ambient php.ini\n");
    fwrite(STDERR, "  entry before the isolated harness could load $extPath.\n");
    fwrite(STDERR, "  preflight stderr:\n");
    fwrite(STDERR, "    " . alef_indent_lines($preflightStderr) . "\n");
    fwrite(STDERR, "This means an unrelated, previously installed copy of the extension is\n");
    fwrite(STDERR, "winning over this checkout's build. Remove or disable it from php.ini /\n");
    fwrite(STDERR, "conf.d before running the suite.\n");
    exit(1);
}
if ($preflightExit !== 0 || $preflightStdout === '') {
    fwrite(STDERR, "error: the __EXTENSION_NAME__ extension did not load from $extPath.\n");
    fwrite(STDERR, "  preflight exit code: $preflightExit\n");
    if ($preflightStderr !== '') {
        fwrite(STDERR, "  preflight stderr:\n");
        fwrite(STDERR, "    " . alef_indent_lines($preflightStderr) . "\n");
    }
    exit(1);
}
$loadedVersion = $preflightStdout;
if ($loadedVersion !== '__PKG_VERSION__') {
    fwrite(STDERR, "error: loaded __EXTENSION_NAME__ extension version mismatch.\n");
    fwrite(STDERR, "  extension at $extPath reports version: $loadedVersion\n");
    fwrite(STDERR, "  expected version: __PKG_VERSION__\n");
    fwrite(STDERR, "Rebuild it with:\n");
    fwrite(STDERR, "  cargo build --release -p __CARGO_PACKAGE_NAME__\n");
    exit(1);
}

// Invoke PHPUnit through the same isolated, verified configuration.
$phpunitPath = __DIR__ . '/vendor/bin/phpunit';
if (!file_exists($phpunitPath)) {
    echo "PHPUnit not found at $phpunitPath. Run 'composer install' first.\n";
    exit(1);
}

$cmd = array_merge(
    [$php],
    $phpConfigArgs,
    [$phpunitPath],
    array_slice($GLOBALS['argv'], 1)
);
passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);
exit($exitCode);
"#;
    TEMPLATE
        .replace("__HEADER__", &header)
        .replace("__EXT_LIB_NAME__", &ext_lib_name)
        .replace("__EXTENSION_NAME__", extension_name)
        .replace("__PKG_VERSION__", pkg_version)
        .replace("__CARGO_PACKAGE_NAME__", cargo_package_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn make_e2e_config_with_env(env: BTreeMap<String, String>) -> E2eConfig {
        E2eConfig {
            env,
            ..E2eConfig::default()
        }
    }

    #[test]
    fn test_render_env_setup_block_empty_env() {
        let config = make_e2e_config_with_env(BTreeMap::new());
        let result = render_env_setup_block(&config);
        assert!(result.is_empty(), "empty env should produce empty setup block");
    }

    #[test]
    fn test_render_env_setup_block_single_env_var() {
        let mut env = BTreeMap::new();
        env.insert("ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());

        let config = make_e2e_config_with_env(env);
        let result = render_env_setup_block(&config);

        assert!(
            result.contains("getenv('ALLOW_PRIVATE_NETWORK')"),
            "should check getenv"
        );
        assert!(
            result.contains("putenv('ALLOW_PRIVATE_NETWORK=true')"),
            "should call putenv"
        );
        assert!(
            result.contains("$_ENV['ALLOW_PRIVATE_NETWORK'] = 'true'"),
            "should set $_ENV"
        );
        assert!(
            result.contains("$_SERVER['ALLOW_PRIVATE_NETWORK'] = 'true'"),
            "should set $_SERVER"
        );
    }

    #[test]
    fn test_render_env_setup_block_multiple_env_vars_sorted() {
        let mut env = BTreeMap::new();
        env.insert("ZEBRA_VAR".to_string(), "z_value".to_string());
        env.insert("ALPHA_VAR".to_string(), "a_value".to_string());
        env.insert("BETA_VAR".to_string(), "b_value".to_string());

        let config = make_e2e_config_with_env(env);
        let result = render_env_setup_block(&config);

        // Check that all variables are present
        assert!(result.contains("ALPHA_VAR"), "should contain ALPHA_VAR");
        assert!(result.contains("BETA_VAR"), "should contain BETA_VAR");
        assert!(result.contains("ZEBRA_VAR"), "should contain ZEBRA_VAR");

        // Check alphabetical ordering by verifying positions
        let alpha_pos = result.find("ALPHA_VAR").unwrap();
        let beta_pos = result.find("BETA_VAR").unwrap();
        let zebra_pos = result.find("ZEBRA_VAR").unwrap();

        assert!(alpha_pos < beta_pos, "ALPHA_VAR should appear before BETA_VAR");
        assert!(beta_pos < zebra_pos, "BETA_VAR should appear before ZEBRA_VAR");
    }

    #[test]
    fn test_render_env_setup_block_special_characters_escaped() {
        let mut env = BTreeMap::new();
        env.insert("PATH_VAR".to_string(), "/some/path/value".to_string());

        let config = make_e2e_config_with_env(env);
        let result = render_env_setup_block(&config);

        assert!(
            result.contains("putenv('PATH_VAR=/some/path/value')"),
            "should preserve path"
        );
    }

    #[test]
    fn test_render_run_tests_php_fails_loudly_when_extension_missing() {
        let result = render_run_tests_php("sample_ext", None, "sample-ext-php", "1.2.3");

        let expected_failure_branch = "\
if (!file_exists($extPath)) {
    fwrite(STDERR, \"error: no sample_ext PHP extension build found.\\n\");
    fwrite(STDERR, \"  looked for a local build at: $localExtPath\\n\");
    $pieDisplay = $pieInstalledExtPath !== false ? $pieInstalledExtPath : '(unset)';
    fwrite(STDERR, \"  looked for PIE_INSTALLED_EXTENSION_PATH at: $pieDisplay\\n\");
    fwrite(STDERR, \"Build it locally with:\\n\");
    fwrite(STDERR, \"  cargo build --release -p sample-ext-php\\n\");
    exit(1);
}";
        assert!(
            result.contains(expected_failure_branch),
            "generated run_tests.php should fail loudly with both looked-up paths and a \
             build hint when no extension is found, got:\n{result}"
        );

        // The old silent-fallthrough guard (only re-exec when the extension exists) must be
        // gone -- the script now always exits above when $extPath is missing, so re-exec is
        // unconditional on ALEF_PHP_EXT_LOADED alone.
        assert!(
            !result.contains("if (file_exists($extPath) && !getenv('ALEF_PHP_EXT_LOADED')) {"),
            "should no longer silently defer to an ambient extension"
        );
    }

    #[test]
    fn test_render_run_tests_php_asserts_preflight_extension_version() {
        let result = render_run_tests_php("sample_ext", None, "sample-ext-php", "1.2.3");

        let expected_version_branch = "\
$loadedVersion = $preflightStdout;
if ($loadedVersion !== '1.2.3') {
    fwrite(STDERR, \"error: loaded sample_ext extension version mismatch.\\n\");
    fwrite(STDERR, \"  extension at $extPath reports version: $loadedVersion\\n\");
    fwrite(STDERR, \"  expected version: 1.2.3\\n\");
    fwrite(STDERR, \"Rebuild it with:\\n\");
    fwrite(STDERR, \"  cargo build --release -p sample-ext-php\\n\");
    exit(1);
}";
        assert!(
            result.contains(expected_version_branch),
            "generated run_tests.php should assert the preflight-reported extension version \
             against the workspace version baked in at generation time, got:\n{result}"
        );
    }

    #[test]
    fn test_render_run_tests_php_detects_already_loaded_warning() {
        let result = render_run_tests_php("sample_ext", None, "sample-ext-php", "1.2.3");

        assert!(
            result.contains("stripos($preflightStderr, 'already loaded') !== false"),
            "generated run_tests.php should scan preflight stderr for an \"already loaded\" \
             startup warning instead of trusting the exit code alone, got:\n{result}"
        );
        assert!(
            result.contains("error: the sample_ext extension is already loaded from an ambient php.ini"),
            "the already-loaded failure message should name the extension and explain the \
             collision, got:\n{result}"
        );
    }

    #[test]
    fn test_render_run_tests_php_builds_isolated_ini_before_loading_extension() {
        let result = render_run_tests_php("sample_ext", None, "sample-ext-php", "1.2.3");

        assert!(
            result.contains("function alef_build_isolated_ini(string $php, string $extensionName): string {"),
            "generated run_tests.php should define a helper that filters the ambient php.ini \
             instead of trusting `-d extension=` alone to win, got:\n{result}"
        );
        assert!(
            result.contains("$isolatedIni = alef_build_isolated_ini($php, 'sample_ext');"),
            "the isolated ini must be built for this generated extension's own name, got:\n{result}"
        );
        assert!(
            result.contains("$phpConfigArgs = ['-n', '-c', $isolatedIni, '-d', 'extension=' . $extPath];"),
            "both PHPUnit invocation and the preflight check must run under `-n -c <isolated ini>` \
             plus an explicit `-d extension=` for the built extension, got:\n{result}"
        );
    }

    #[test]
    fn test_render_run_tests_php_preflight_runs_unconditionally_before_phpunit() {
        let result = render_run_tests_php("sample_ext", None, "sample-ext-php", "1.2.3");

        // Regression guard: a previous revision put the version/identity check after a
        // branch that unconditionally `passthru`'d into PHPUnit and then `exit`'d, so the
        // check could never execute. The preflight `proc_open` call (and the version check
        // that follows it) must appear strictly BEFORE the PHPUnit `passthru` invocation, and
        // no unconditional `exit($exitCode)` may sit between the start of the file and the
        // preflight version check.
        let preflight_pos = result
            .find("$process = proc_open(")
            .expect("preflight proc_open call must be present");
        let version_check_pos = result
            .find("if ($loadedVersion !== '1.2.3') {")
            .expect("preflight version check must be present");
        let passthru_pos = result
            .find("passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);")
            .expect("PHPUnit passthru invocation must be present");

        assert!(
            preflight_pos < version_check_pos,
            "preflight identity check must run before the version comparison, got:\n{result}"
        );
        assert!(
            version_check_pos < passthru_pos,
            "the version check must run before PHPUnit is ever invoked, so it cannot be \
             stranded after an unconditional exit like the previous defect, got:\n{result}"
        );

        // The old re-exec gate (`ALEF_PHP_EXT_LOADED`) that caused the guard to be
        // unreachable must not reappear.
        assert!(
            !result.contains("ALEF_PHP_EXT_LOADED"),
            "the dead re-exec gate must not reappear -- the preflight check must be reached by \
             plain sequential execution of this single script, got:\n{result}"
        );
    }
}
