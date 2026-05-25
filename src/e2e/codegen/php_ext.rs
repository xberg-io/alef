//! PHP native extension (php-ext / PIE) test_app generator.
//!
//! Generates a registry-mode-only test_app at `test_apps/php_ext/` that
//! installs the sample-markdown PHP native extension via PIE and exercises
//! the raw C extension functions.
//!
//! Emits three files:
//!
//! - `run_tests.sh` — installs the PIE extension and runs `main.php`.
//! - `main.php` — verifies extension loading and calls `sample_markdown_convert`.
//! - `README.md` — describes the test_app.
//!
//! This generator is registry-mode only.  In local mode it emits a single
//! stub `README.md` explaining why generation was skipped.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::{E2eCodegen, TestBackendEmission};
use crate::core::config::e2e::DependencyMode;

/// PHP native extension (PIE) test_app generator.
pub struct PhpExtCodegen;

impl E2eCodegen for PhpExtCodegen {
    fn generate(
        &self,
        _groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        if e2e_config.dep_mode != DependencyMode::Registry {
            // Local mode: emit a stub README only.
            return Ok(vec![GeneratedFile {
                path: output_base.join("README.md"),
                content: stub_readme(),
                generated_header: false,
            }]);
        }

        // Resolve package config.
        let pkg = e2e_config.registry.packages.get(lang);
        let pkg_name = pkg
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("example/{}-ext", config.name));
        let version = pkg
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        let extension_name = config.php_extension_name();

        Ok(vec![
            GeneratedFile {
                path: output_base.join("run_tests.sh"),
                content: render_run_tests(&pkg_name, &version, &extension_name),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("main.php"),
                content: render_main_php(&extension_name),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("README.md"),
                content: render_readme(&pkg_name, &version),
                generated_header: false,
            },
        ])
    }

    fn language_name(&self) -> &'static str {
        "php_ext"
    }
}

/// Stub README emitted in local mode.
fn stub_readme() -> String {
    "# php-ext test_app\n\nThis test_app is registry-mode only.\n\
     Run `alef e2e generate --registry` (or `alef test-apps generate`) to generate it.\n"
        .to_string()
}

/// Render `run_tests.sh`.
fn render_run_tests(pkg_name: &str, version: &str, extension_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# Installs the PIE PHP native extension and runs main.php.");
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let _ = writeln!(out, "VERSION=\"{version}\"");
    let _ = writeln!(out, "PKG=\"{pkg_name}\"");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "SCRIPT_DIR=\"$(cd \"$(dirname \"${{BASH_SOURCE[0]}}\")\" && pwd)\""
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "# Require PIE.");
    let _ = writeln!(out, "if ! command -v pie &>/dev/null; then");
    let _ = writeln!(
        out,
        "  echo 'error: pie is required. Install via: composer global require php/pie' >&2"
    );
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"Installing $PKG version $VERSION via PIE...\"");
    let _ = writeln!(out, "pie install \"$PKG\" --version \"$VERSION\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Locate the installed extension.");
    let _ = writeln!(out, "EXT_DIR=\"$(php -r 'echo ini_get(\"extension_dir\");')\"");
    let _ = writeln!(out, "EXT_NAME=\"{extension_name}\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Determine OS-specific extension suffix.");
    let _ = writeln!(out, "case \"$(uname -s)\" in");
    let _ = writeln!(out, "  Darwin) EXT_SUFFIX=\".dylib\" ;;");
    let _ = writeln!(out, "  *)      EXT_SUFFIX=\".so\" ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "EXT_PATH=\"$EXT_DIR/$EXT_NAME$EXT_SUFFIX\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "if [ ! -f \"$EXT_PATH\" ]; then");
    let _ = writeln!(out, "  echo \"error: extension not found at $EXT_PATH\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"Running main.php with extension=$EXT_PATH ...\"");
    let _ = writeln!(out, "php -d \"extension=$EXT_PATH\" \"$SCRIPT_DIR/main.php\"");
    out
}

/// Render `main.php`.
fn render_main_php(extension_name: &str) -> String {
    let mut out = String::new();
    let convert_function = format!("{extension_name}_convert");
    let header = hash::header(CommentStyle::DoubleSlash);
    out.push_str("<?php\n\n");
    out.push_str(&header);
    let _ = writeln!(out);
    let _ = writeln!(out, "declare(strict_types=1);");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Verify the extension is loaded.");
    let _ = writeln!(out, "if (!extension_loaded('{extension_name}')) {{");
    let _ = writeln!(
        out,
        "    fwrite(STDERR, \"FAIL: {extension_name} extension is not loaded\\n\");"
    );
    let _ = writeln!(out, "    exit(1);");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Verify the convert function exists.");
    let _ = writeln!(out, "if (!function_exists('{convert_function}')) {{");
    let _ = writeln!(
        out,
        "    fwrite(STDERR, \"FAIL: {convert_function}() not found\\n\");"
    );
    let _ = writeln!(out, "    exit(1);");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Smoke-test: convert a heading.");
    let _ = writeln!(out, "$result = {convert_function}('<h1>Hi</h1>');");
    let _ = writeln!(out);
    let _ = writeln!(out, "if (!str_contains($result, 'Hi')) {{");
    let _ = writeln!(
        out,
        "    fwrite(STDERR, \"FAIL: expected output to contain 'Hi', got: $result\\n\");"
    );
    let _ = writeln!(out, "    exit(1);");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "echo \"PASS: {convert_function}('<h1>Hi</h1>') => $result\\n\";"
    );
    let _ = writeln!(out, "exit(0);");
    out
}

/// Render `README.md`.
fn render_readme(pkg_name: &str, version: &str) -> String {
    format!(
        "# php-ext test_app\n\n\
         Exercises the configured PHP native extension (`{pkg_name}` v`{version}`)\n\
         installed via [PIE](https://github.com/php/pie).\n\n\
         ## Running\n\n\
         ```bash\n\
         bash run_tests.sh\n\
         ```\n\n\
         ## What it tests\n\n\
         - PIE installs the extension successfully.\n\
         - The extension loads successfully.\n\
         - The configured convert function returns a string containing `Hi`.\n"
    )
}

/// Emit a test backend stub (not applicable for php_ext).
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    TestBackendEmission::unimplemented("php_ext")
}
