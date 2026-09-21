//! PHP native extension (php-ext / PIE) test_app generator.
//!
//! Generates a registry-mode-only test_app at `test_apps/php_ext/` that
//! installs the configured PHP native extension via PIE and exercises
//! the configured e2e call's facade static method when e2e call config is available.
//! The php-ext backend places crate-level free functions on a namespaced facade class
//! rather than emitting global functions, so the smoke call goes through that class
//! (see `backends::php::naming::php_ext_api_class_name`).
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
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
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
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
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
        let smoke_call = resolve_smoke_call(e2e_config, config, &extension_name);

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

    /// php_ext has no documentation-snippet recipe at all -- `render_snippet_body` is
    /// unimplemented here and falls to [`E2eCodegen::render_snippet_body`]'s own "does not
    /// support documentation snippets" error, for every fixture, always. Forwarding to it
    /// explicitly states that as a checked choice rather than an inherited default: php_ext
    /// never reaches `CallIr::signature` through this path, so it structurally cannot have the
    /// kotlin_android bug this override exists to rule out. See
    /// [`E2eCodegen::render_snippet_body_with_functions`]'s doc comment. ~keep
    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        self.render_snippet_body(fixture, e2e_config, config, type_defs, enums)
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
    out.push_str(&hash::e2e_header(CommentStyle::Hash));
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
    /// Fully-qualified PHP class name (namespace + facade class, no leading backslash).
    class_name: String,
    /// The static method's PHP name (lowerCamelCase).
    method_name: String,
    argument: Option<String>,
}

/// Resolve the configured e2e call into a facade static-method reference.
///
/// The php-ext backend never emits crate-level free functions as global `#[php_function]`
/// items — it always places them as static methods on a namespaced facade class instead (see
/// `backends::php::naming::php_ext_api_class_name`). The smoke call must be routed the same
/// way, or it probes a symbol the generated extension can never provide.
fn resolve_smoke_call(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    extension_name: &str,
) -> Option<PhpExtSmokeCall> {
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

    let namespace = crate::backends::php::naming::php_autoload_namespace(config);
    let api_class = crate::backends::php::naming::php_ext_api_class_name(extension_name);
    let class_name = format!("{namespace}\\{api_class}");
    let method_name = crate::codegen::naming::to_php_name(configured_name);

    let argument = e2e_config
        .call
        .args
        .first()
        .and_then(|arg| match arg.arg_type.as_str() {
            "string" | "bytes" | "file_path" => Some("smoke test".to_string()),
            _ => None,
        });

    Some(PhpExtSmokeCall {
        class_name,
        method_name,
        argument,
    })
}

/// Render `main.php`.
fn render_main_php(extension_name: &str, smoke_call: Option<&PhpExtSmokeCall>) -> String {
    let mut out = String::new();
    let header = hash::e2e_header(CommentStyle::DoubleSlash);
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
        let class_name = &call.class_name;
        let method_name = &call.method_name;
        let call_label = format!("{class_name}::{method_name}");
        let _ = writeln!(out);
        let _ = writeln!(out, "// Verify the configured facade method exists.");
        let _ = writeln!(out, "if (!class_exists('{class_name}')) {{");
        let _ = writeln!(out, "    fwrite(STDERR, \"FAIL: class {class_name} not found\\n\");");
        let _ = writeln!(out, "    exit(1);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out, "if (!method_exists('{class_name}', '{method_name}')) {{");
        let _ = writeln!(out, "    fwrite(STDERR, \"FAIL: {call_label}() not found\\n\");");
        let _ = writeln!(out, "    exit(1);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "// Smoke-test the configured facade method.");
        if let Some(argument) = &call.argument {
            let escaped = argument.replace('\\', "\\\\").replace('\'', "\\'");
            let _ = writeln!(out, "$result = {call_label}('{escaped}');");
        } else {
            let _ = writeln!(out, "$result = {call_label}();");
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "if ($result === null) {{");
        let _ = writeln!(out, "    fwrite(STDERR, \"FAIL: expected non-null result\\n\");");
        let _ = writeln!(out, "    exit(1);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "echo \"PASS: {call_label}() returned a non-null result\\n\";");
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
///
/// php_ext is a packaging-verification backend with no `test_backend` stub
/// generator. Panic rather than return a placeholder `TestBackendEmission` a
/// caller could accidentally splice into generated code. ~keep
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    panic!(
        "php_ext e2e generator: fixture `{}` requires a php_ext test_backend stub for trait `{}`, but the php_ext test-backend emitter is unimplemented; refusing to emit a call with a comment where the argument belongs",
        fixture.id, trait_bridge.trait_name
    );
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
    fn main_php_with_call_config_checks_configured_facade_method() {
        let smoke_call = PhpExtSmokeCall {
            class_name: "Demo\\Ext\\DemoExtApi".to_string(),
            method_name: "render".to_string(),
            argument: Some("smoke test".to_string()),
        };

        let content = render_main_php("demo_ext", Some(&smoke_call));

        assert!(content.contains("class_exists('Demo\\Ext\\DemoExtApi')"));
        assert!(content.contains("method_exists('Demo\\Ext\\DemoExtApi', 'render')"));
        assert!(content.contains("$result = Demo\\Ext\\DemoExtApi::render('smoke test');"));
        assert!(content.contains("expected non-null result"));
        assert!(!content.contains("function_exists("));
    }
}
