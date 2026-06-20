//! PHP native extension (php-ext / PIE) test_app generator.
//!
//! Generates a registry-mode-only test_app at `test_apps/php_ext/` that
//! installs the configured PHP native extension via PIE and exercises
//! the raw C extension functions when e2e call config is available.
//!
//! Emits three files:
//!
//! - `run_tests.sh` — installs the PIE extension and runs `main.php`.
//! - `main.php` — verifies extension loading and optionally calls the configured function.
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

        // Resolve package config. Try php_ext first, fall back to regular PHP package,
        // then derive from call.module (stripping -rs suffix, as per Packagist naming conventions).
        let pkg_ext = e2e_config.resolve_package(lang);
        let pkg_php = e2e_config.resolve_package("php");

        let pkg_name = pkg_ext
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .or_else(|| pkg_php.as_ref().and_then(|p| p.name.as_ref()).cloned())
            .unwrap_or_else(|| {
                let org = config
                    .try_github_repo()
                    .ok()
                    .as_deref()
                    .and_then(crate::core::config::derive_repo_org)
                    .unwrap_or_else(|| config.name.clone());
                let mut pkg_module = e2e_config.call.module.replace('_', "-");
                // Strip Rust FFI crate suffix for Packagist package naming convention.
                if pkg_module.ends_with("-rs") {
                    pkg_module = pkg_module[..pkg_module.len() - 3].to_string();
                }
                format!("{org}/{pkg_module}")
            });
        let version = pkg_ext
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| pkg_php.as_ref().and_then(|p| p.version.as_ref()).cloned())
            .unwrap_or_else(|| "0.1.0".to_string());

        let extension_name = config.php_extension_name();
        let smoke_call = resolve_smoke_call(e2e_config, &extension_name);

        Ok(vec![
            GeneratedFile {
                path: output_base.join("run_tests.sh"),
                content: render_run_tests(&pkg_name, &version, &extension_name),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("main.php"),
                content: render_main_php(&extension_name, smoke_call.as_ref()),
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
    // PIE's `install` has no `--version` option (it parses `--version`/`-V` as
    // "print PIE's own version" and exits without installing). The version is part
    // of the package coordinate: `vendor/package:constraint`.
    let _ = writeln!(out, "pie install \"$PKG:$VERSION\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Locate the installed extension.");
    let _ = writeln!(out, "EXT_DIR=\"$(php -r 'echo ini_get(\"extension_dir\");')\"");
    let _ = writeln!(out, "EXT_NAME=\"{extension_name}\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Determine OS-specific extension suffix.");
    let _ = writeln!(out, "case \"$(uname -s)\" in");
    let _ = writeln!(out, "Darwin) EXT_SUFFIX=\".dylib\" ;;");
    let _ = writeln!(out, "*) EXT_SUFFIX=\".so\" ;;");
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

struct PhpExtSmokeCall {
    function_name: String,
    argument: Option<String>,
}

fn resolve_smoke_call(e2e_config: &E2eConfig, extension_name: &str) -> Option<PhpExtSmokeCall> {
    let configured_name = e2e_config
        .call
        .overrides
        .get("php_ext")
        .and_then(|override_config| override_config.function.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get("php")
                .and_then(|override_config| override_config.function.as_deref())
        })
        .or_else(|| (!e2e_config.call.function.is_empty()).then_some(e2e_config.call.function.as_str()))?;

    let function_name = if configured_name.starts_with(extension_name) {
        configured_name.to_string()
    } else {
        format!("{extension_name}_{configured_name}")
    };

    let argument = e2e_config
        .call
        .args
        .first()
        .and_then(|arg| match arg.arg_type.as_str() {
            "string" | "bytes" | "file_path" => Some("smoke test".to_string()),
            _ => None,
        });

    Some(PhpExtSmokeCall {
        function_name,
        argument,
    })
}

/// Render `main.php`.
fn render_main_php(extension_name: &str, smoke_call: Option<&PhpExtSmokeCall>) -> String {
    let mut out = String::new();
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
    if let Some(call) = smoke_call {
        let function_name = &call.function_name;
        let _ = writeln!(out);
        let _ = writeln!(out, "// Verify the configured function exists.");
        let _ = writeln!(out, "if (!function_exists('{function_name}')) {{");
        let _ = writeln!(out, "    fwrite(STDERR, \"FAIL: {function_name}() not found\\n\");");
        let _ = writeln!(out, "    exit(1);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "// Smoke-test the configured function.");
        if let Some(argument) = &call.argument {
            let escaped = argument.replace('\\', "\\\\").replace('\'', "\\'");
            let _ = writeln!(out, "$result = {function_name}('{escaped}');");
        } else {
            let _ = writeln!(out, "$result = {function_name}();");
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "if ($result === null) {{");
        let _ = writeln!(out, "    fwrite(STDERR, \"FAIL: expected non-null result\\n\");");
        let _ = writeln!(out, "    exit(1);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "echo \"PASS: {function_name}() returned a non-null result\\n\";");
    } else {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "echo \"PASS: {extension_name} extension loaded; no e2e call configured\\n\";"
        );
    }
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
         - The configured e2e call function, when present, returns a non-null value.\n"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_php_without_call_config_only_checks_extension_loaded() {
        let content = render_main_php("demo_ext", None);

        assert!(content.contains("extension_loaded('demo_ext')"));
        assert!(content.contains("no e2e call configured"));
        assert!(!content.contains("demo_ext_convert"));
        assert!(!content.contains("<h1>Hi</h1>"));
    }

    #[test]
    fn main_php_with_call_config_checks_configured_function() {
        let smoke_call = PhpExtSmokeCall {
            function_name: "demo_ext_render".to_string(),
            argument: Some("smoke test".to_string()),
        };

        let content = render_main_php("demo_ext", Some(&smoke_call));

        assert!(content.contains("function_exists('demo_ext_render')"));
        assert!(content.contains("$result = demo_ext_render('smoke test');"));
        assert!(content.contains("expected non-null result"));
    }
}
