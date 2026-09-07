//! Homebrew test_app generator.
//!
//! Generates a registry-mode-only test_app at `test_apps/homebrew/` that
//! exercises a Homebrew tap's CLI formula and (optionally) FFI formula.
//!
//! Emitted files:
//!
//! - `Brewfile` — declares tap + formulae for `brew bundle install`.
//! - `run_tests.sh` — installs via Brewfile and runs the configured CLI tests.
//!   When `ffi_formula` is set it also compiles and runs `ffi_smoke.c`.
//! - `ffi_smoke.c` — minimal C program linking against the FFI formula.
//!   Only emitted when `ffi_formula` is configured.
//! - `README.md` — describes the test_app.
//!
//! This generator is registry-mode only.  In local mode it emits a single
//! stub `README.md` explaining why generation was skipped.
//!
//! # Configuration (`alef.toml`)
//!
//! ```toml
//! [crates.e2e.registry.packages.homebrew]
//! tap = "myorg/tap"
//! cli_formula = "my-tool"
//! # ffi_formula is optional; omit to suppress FFI sections entirely.
//! ffi_formula = "libmy-tool"
//!
//! # Optional: explicit CLI tests.  When absent, a single --version check is emitted.
//! [[crates.e2e.registry.packages.homebrew.cli_tests]]
//! name = "version"
//! command = "$CLI_FORMULA --version"
//! expect_contains = "1.2.3"
//! ```

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::HomebrewCliTest;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use crate::core::config::e2e::DependencyMode;

/// Homebrew formula test_app generator.
pub struct HomebrewCodegen;

impl E2eCodegen for HomebrewCodegen {
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
            return Ok(vec![GeneratedFile {
                path: output_base.join("README.md"),
                content: stub_readme(),
                generated_header: false,
            }]);
        }

        // Resolve Homebrew-specific package config.
        let pkg = e2e_config.registry.packages.get(lang);
        let tap = pkg
            .and_then(|p| p.tap.as_ref())
            .cloned()
            .unwrap_or_else(|| "example/tap".to_string());
        let cli_formula = pkg
            .and_then(|p| p.cli_formula.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.clone());
        // ffi_formula is purely opt-in: no defaulting to `lib{cli_formula}`.
        let ffi_formula: Option<String> = pkg.and_then(|p| p.ffi_formula.clone());
        // Version: prefer explicit package config, fall back to crate version, then placeholder.
        let version = pkg
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());
        // CLI tests: use explicit config; when empty emit a single --version default.
        let cli_tests: Vec<HomebrewCliTest> = match pkg {
            Some(p) if !p.cli_tests.is_empty() => p.cli_tests.clone(),
            _ => default_cli_tests(&version),
        };

        let mut files = vec![
            GeneratedFile {
                path: output_base.join("Brewfile"),
                content: render_brewfile(&tap, &cli_formula, ffi_formula.as_deref()),
                generated_header: false,
            },
            GeneratedFile {
                path: output_base.join("run_tests.sh"),
                content: render_run_tests(
                    &tap,
                    &cli_formula,
                    ffi_formula.as_deref(),
                    &version,
                    config.ffi_lib_name().as_str(),
                    &cli_tests,
                ),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("README.md"),
                content: render_readme(&tap, &cli_formula, ffi_formula.as_deref(), &version, &cli_tests),
                generated_header: false,
            },
        ];

        // ffi_smoke.c is only emitted when an FFI formula is configured.
        if let Some(ref ffi) = ffi_formula {
            let ffi_header = config.ffi_header_name();
            let ffi_prefix = config.ffi_prefix();
            files.push(GeneratedFile {
                path: output_base.join("ffi_smoke.c"),
                content: render_ffi_smoke_c(&ffi_header, &ffi_prefix, ffi),
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "homebrew"
    }

    /// Homebrew has no documentation-snippet recipe at all -- `render_snippet_body` is
    /// unimplemented here and falls to [`E2eCodegen::render_snippet_body`]'s own "does not
    /// support documentation snippets" error, for every fixture, always. Forwarding to it
    /// explicitly states that as a checked choice rather than an inherited default: homebrew
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

/// Default CLI tests when none are configured: a single `--version` check.
fn default_cli_tests(version: &str) -> Vec<HomebrewCliTest> {
    vec![HomebrewCliTest {
        name: "version".to_string(),
        command: "$CLI_FORMULA --version".to_string(),
        expect_contains: Some(version.to_string()),
    }]
}

/// Stub README emitted in local mode.
fn stub_readme() -> String {
    "# homebrew test_app\n\nThis test_app is registry-mode only.\n\
     Run `alef e2e generate --registry` (or `alef test-apps generate`) to generate it.\n"
        .to_string()
}

/// Render `Brewfile`.
///
/// Formulae are emitted with their fully-qualified `tap/formula` names so
/// `brew bundle install` works even when other taps installed on the
/// developer's machine expose colliding formula short-names.
fn render_brewfile(tap: &str, cli_formula: &str, ffi_formula: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Brewfile — managed by alef. DO NOT EDIT.");
    let _ = writeln!(out, "tap {}", crate::e2e::escape::ruby_string_literal(tap));
    let formula = format!("{tap}/{cli_formula}");
    let _ = writeln!(out, "brew {}", crate::e2e::escape::ruby_string_literal(&formula));
    if let Some(ffi) = ffi_formula {
        let formula = format!("{tap}/{ffi}");
        let _ = writeln!(out, "brew {}", crate::e2e::escape::ruby_string_literal(&formula));
    }
    out
}

/// Render `run_tests.sh`.
fn render_run_tests(
    tap: &str,
    cli_formula: &str,
    ffi_formula: Option<&str>,
    version: &str,
    ffi_lib_name: &str,
    cli_tests: &[HomebrewCliTest],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(
        out,
        "# Tests the Homebrew CLI formula{}.",
        if ffi_formula.is_some() { " and FFI formula" } else { "" }
    );
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let quote = crate::core::config::shell::quote_word;
    let _ = writeln!(out, "VERSION={}", quote(version));
    let _ = writeln!(out, "TAP={}", quote(tap));
    let _ = writeln!(out, "CLI_FORMULA={}", quote(cli_formula));
    if let Some(ffi) = ffi_formula {
        let _ = writeln!(out, "FFI_FORMULA={}", quote(ffi));
        // Fully-qualified name disambiguates when multiple taps export the same
        // formula short-name on a developer's machine.
        let _ = writeln!(out, "FFI_FORMULA_QUALIFIED=\"$TAP/$FFI_FORMULA\"");
    }
    let _ = writeln!(
        out,
        "SCRIPT_DIR=\"$(cd \"$(dirname \"${{BASH_SOURCE[0]}}\")\" && pwd)\""
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "PASS=0");
    let _ = writeln!(out, "FAIL=0");
    let _ = writeln!(out);

    // Helper functions.
    let _ = writeln!(out, "pass() {{");
    let _ = writeln!(out, "  echo \"PASS: $1\"");
    let _ = writeln!(out, "  PASS=$((PASS + 1))");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "fail() {{");
    let _ = writeln!(out, "  echo \"FAIL: $1 — $2\" >&2");
    let _ = writeln!(out, "  FAIL=$((FAIL + 1))");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Install step.
    let _ = writeln!(out, "# Trust and install formulae.");
    let _ = writeln!(out, "brew trust \"$TAP\" || true");
    let _ = writeln!(out, "brew bundle install --file=\"$SCRIPT_DIR/Brewfile\"");
    let _ = writeln!(out);

    // Emit each CLI test.
    for test in cli_tests {
        let suffix = sanitize_var_name(&test.name);
        let _ = writeln!(out, "# Test: {}.", test.name.replace(['\r', '\n'], " "));
        let _ = writeln!(out, "_command_{suffix}={}", quote(&test.command));
        let _ = writeln!(out, "_output_{suffix}=$(eval \"$_command_{suffix}\" 2>&1 || true)");
        if let Some(ref expected) = test.expect_contains {
            let _ = writeln!(out, "_expected_{suffix}={}", quote(expected));
            let _ = writeln!(out, "if [[ \"$_output_{suffix}\" == *\"$_expected_{suffix}\"* ]]; then");
            let _ = writeln!(out, "  pass {}", quote(&test.name));
            let _ = writeln!(out, "else");
            let _ = writeln!(
                out,
                "  fail {} \"expected '$_expected_{suffix}' in output, got: $_output_{suffix}\"",
                quote(&test.name)
            );
            let _ = writeln!(out, "fi");
        } else {
            // No expected substring — just require zero exit code.
            let _ = writeln!(
                out,
                "_exit_{suffix}=$(eval \"$_command_{suffix}\" >/dev/null 2>&1; echo $?)"
            );
            let _ = writeln!(out, "if [ \"$_exit_{suffix}\" -eq 0 ]; then");
            let _ = writeln!(out, "  pass {}", quote(&test.name));
            let _ = writeln!(out, "else");
            let _ = writeln!(
                out,
                "  fail {} \"command exited with code $_exit_{suffix}\"",
                quote(&test.name)
            );
            let _ = writeln!(out, "fi");
        }
        let _ = writeln!(out);
    }

    // FFI section (only when ffi_formula is set).
    if ffi_formula.is_some() {
        let _ = writeln!(out, "# Test: FFI formula — compile and run ffi_smoke.c.");
        let _ = writeln!(out, "TMP_DIR=$(mktemp -d)");
        let _ = writeln!(out, "trap 'rm -rf \"$TMP_DIR\"' EXIT");
        let _ = writeln!(out);
        // pkg-config .pc filename is determined by the upstream FFI crate
        // and does not necessarily match the brew formula name. Try
        // pkg-config first; if it produces no flags, fall back to
        // brew --prefix for canonical paths.
        let _ = writeln!(out, "FFI_CFLAGS=\"\"");
        let _ = writeln!(out, "FFI_LIBS=\"\"");
        let _ = writeln!(out, "if command -v pkg-config &>/dev/null; then");
        let _ = writeln!(
            out,
            "  FFI_CFLAGS=$(pkg-config --cflags \"$FFI_FORMULA\" 2>/dev/null || true)"
        );
        let _ = writeln!(
            out,
            "  FFI_LIBS=$(pkg-config --libs \"$FFI_FORMULA\" 2>/dev/null || true)"
        );
        let _ = writeln!(out, "fi");
        let _ = writeln!(out, "if [[ -z \"${{FFI_CFLAGS:-}}\" ]]; then");
        let _ = writeln!(out, "  # Fallback: use brew --prefix to locate headers and libs.");
        let _ = writeln!(
            out,
            "  FFI_PREFIX=$(brew --prefix \"$FFI_FORMULA_QUALIFIED\" 2>/dev/null || true)"
        );
        let _ = writeln!(out, "  FFI_CFLAGS=\"-I$FFI_PREFIX/include\"");
        let _ = writeln!(out, "  FFI_LIB_NAME={}", quote(ffi_lib_name));
        let _ = writeln!(out, "  FFI_LIBS=\"-L$FFI_PREFIX/lib -l${{FFI_LIB_NAME}}\"");
        let _ = writeln!(out, "fi");
        let _ = writeln!(out);
        // shellcheck disable comment for intentional word-splitting on CFLAGS/LIBS.
        let _ = writeln!(out, "# shellcheck disable=SC2086");
        let _ = writeln!(
            out,
            "if cc $FFI_CFLAGS -o \"$TMP_DIR/ffi_smoke\" \"$SCRIPT_DIR/ffi_smoke.c\" $FFI_LIBS 2>&1; then"
        );
        let _ = writeln!(out, "  if \"$TMP_DIR/ffi_smoke\"; then");
        let _ = writeln!(out, "    pass \"ffi-smoke\"");
        let _ = writeln!(out, "  else");
        let _ = writeln!(out, "    fail \"ffi-smoke\" \"ffi_smoke binary exited non-zero\"");
        let _ = writeln!(out, "  fi");
        let _ = writeln!(out, "else");
        let _ = writeln!(out, "  fail \"ffi-smoke\" \"compilation of ffi_smoke.c failed\"");
        let _ = writeln!(out, "fi");
        let _ = writeln!(out);
    }

    // Summary.
    let _ = writeln!(out, "echo \"\"");
    let _ = writeln!(out, "echo \"Results: $PASS passed, $FAIL failed\"");
    let _ = writeln!(out, "[ \"$FAIL\" -eq 0 ]");
    out
}

/// Convert a test name into a valid bash variable name suffix.
///
/// Replaces hyphens and spaces with underscores; strips other non-alphanumeric
/// characters.
fn sanitize_var_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// Render `ffi_smoke.c`.
///
/// The smoke test calls `{ffi_prefix}_version()`, which every FFI library
/// exposes as `const char *` — a pointer to a static version string.  It
/// does **not** return the domain-specific `{Prefix}Result` struct, so the
/// smoke C code must match that exact ABI: declare the return as
/// `const char *`, check for NULL / empty, and print without freeing (the
/// string is statically allocated inside the shared library).
fn render_ffi_smoke_c(ffi_header: &str, ffi_prefix: &str, _ffi_formula: &str) -> String {
    let mut out = String::new();
    let version_fn = format!("{ffi_prefix}_version");
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#include <{ffi_header}>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "  const char *version = {version_fn}();");
    let _ = writeln!(out);
    let _ = writeln!(out, "  if (!version || strlen(version) == 0) {{");
    let _ = writeln!(
        out,
        "    fprintf(stderr, \"FAIL: expected non-empty version string\\n\");"
    );
    let _ = writeln!(out, "    return 1;");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  printf(\"PASS: {version_fn} returned: %s\\n\", version);");
    let _ = writeln!(out, "  return 0;");
    let _ = writeln!(out, "}}");
    out
}

/// Render `README.md`.
fn render_readme(
    tap: &str,
    cli_formula: &str,
    ffi_formula: Option<&str>,
    version: &str,
    cli_tests: &[HomebrewCliTest],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# homebrew test_app");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Exercises the configured Homebrew formulae from tap `{tap}` at version `{version}`."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| Formula | Purpose |");
    let _ = writeln!(out, "|---------|--------|");
    let _ = writeln!(out, "| `{cli_formula}` | CLI binary |");
    if let Some(ffi) = ffi_formula {
        let _ = writeln!(
            out,
            "| `{ffi}` | Shared library: C FFI for embedding in other languages |"
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Running");
    let _ = writeln!(out);
    let _ = writeln!(out, "```bash");
    let _ = writeln!(out, "bash run_tests.sh");
    let _ = writeln!(out, "```");
    let _ = writeln!(out);
    let _ = writeln!(out, "## What it tests");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "1. `brew bundle install` succeeds (tap + formulae install without error)."
    );
    for (i, test) in cli_tests.iter().enumerate() {
        let label = if let Some(ref exp) = test.expect_contains {
            format!("`{}` — output contains `{}`", test.command, exp)
        } else {
            format!("`{}` — exits with code 0", test.command)
        };
        let _ = writeln!(out, "{}. {}.", i + 2, label);
    }
    if ffi_formula.is_some() {
        let next = cli_tests.len() + 2;
        let _ = writeln!(
            out,
            "{next}. `ffi_smoke.c` compiles against the FFI formula (via `pkg-config`) and the \
             compiled binary calls `_version()` successfully."
        );
    }
    out
}

/// Emit a test backend stub (not applicable for homebrew).
///
/// Homebrew is a packaging-verification backend with no `test_backend` stub
/// generator. Panic rather than return a placeholder `TestBackendEmission` a
/// caller could accidentally splice into generated code. ~keep
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    panic!(
        "homebrew e2e generator: fixture `{}` requires a homebrew test_backend stub for trait `{}`, but the homebrew test-backend emitter is unimplemented; refusing to emit a call with a comment where the argument belongs",
        fixture.id, trait_bridge.trait_name
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_test(name: &str, cmd: &str, expect: Option<&str>) -> HomebrewCliTest {
        HomebrewCliTest {
            name: name.to_string(),
            command: cmd.to_string(),
            expect_contains: expect.map(str::to_string),
        }
    }

    // --- default_cli_tests ---

    #[test]
    fn default_cli_tests_emits_version_check() {
        let tests = default_cli_tests("1.2.3");
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "version");
        assert_eq!(tests[0].command, "$CLI_FORMULA --version");
        assert_eq!(tests[0].expect_contains.as_deref(), Some("1.2.3"));
    }

    // --- sanitize_var_name ---

    #[test]
    fn sanitize_var_name_replaces_hyphens_and_spaces() {
        assert_eq!(sanitize_var_name("cli-version"), "cli_version");
        assert_eq!(sanitize_var_name("my test"), "my_test");
        assert_eq!(sanitize_var_name("abc123"), "abc123");
    }

    // --- render_brewfile ---

    #[test]
    fn render_brewfile_without_ffi_omits_ffi_line() {
        let out = render_brewfile("myorg/tap", "mytool", None);
        assert!(out.contains("tap 'myorg/tap'"));
        assert!(out.contains("brew 'myorg/tap/mytool'"));
        assert!(!out.contains("libmytool"), "must not emit a default lib formula");
    }

    #[test]
    fn render_brewfile_with_ffi_includes_ffi_line() {
        let out = render_brewfile("myorg/tap", "mytool", Some("libmytool"));
        assert!(out.contains("brew 'myorg/tap/mytool'"));
        assert!(out.contains("brew 'myorg/tap/libmytool'"));
    }

    // --- render_run_tests without FFI ---

    #[test]
    fn render_run_tests_no_ffi_omits_ffi_section() {
        let tests = default_cli_tests("1.0.0");
        let out = render_run_tests("myorg/tap", "mytool", None, "1.0.0", "mytool", &tests);
        assert!(
            !out.contains("FFI_FORMULA"),
            "must not reference FFI_FORMULA when ffi is None"
        );
        assert!(
            !out.contains("ffi_smoke"),
            "must not reference ffi_smoke when ffi is None"
        );
        assert!(
            !out.contains("pkg-config"),
            "must not reference pkg-config when ffi is None"
        );
    }

    #[test]
    fn render_run_tests_no_ffi_includes_cli_version_check() {
        let tests = default_cli_tests("1.2.3");
        let out = render_run_tests("myorg/tap", "mytool", None, "1.2.3", "mytool", &tests);
        assert!(out.contains("VERSION='1.2.3'"));
        assert!(out.contains("CLI_FORMULA='mytool'"));
        assert!(out.contains("_expected_version='1.2.3'"));
    }

    // --- render_run_tests with FFI ---

    #[test]
    fn render_run_tests_with_ffi_includes_ffi_section() {
        let tests = default_cli_tests("1.0.0");
        let out = render_run_tests("myorg/tap", "mytool", Some("libmytool"), "1.0.0", "mytool", &tests);
        assert!(out.contains("FFI_FORMULA='libmytool'"));
        assert!(out.contains("ffi_smoke"));
        assert!(out.contains("pkg-config"));
    }

    // --- custom cli_tests substitution ---

    #[test]
    fn render_run_tests_custom_cli_tests_emitted_in_order() {
        let tests = vec![
            cli_test("convert", "echo '<h1>Hi</h1>' | $CLI_FORMULA", Some("# Hi")),
            cli_test("help", "$CLI_FORMULA --help", None),
        ];
        let out = render_run_tests("o/t", "tool", None, "0.2.0", "tool", &tests);
        // Both test names appear.
        assert!(out.contains("# Test: convert"));
        assert!(out.contains("# Test: help"));
        // convert uses expect_contains, help uses exit-code check.
        assert!(out.contains("# Hi"));
        // The no-expect_contains branch emits an exit-code variable.
        assert!(out.contains("_exit_help"));
    }

    // --- render_run_tests without expect_contains uses exit code check ---

    #[test]
    fn render_run_tests_no_expect_uses_exit_code_check() {
        let tests = vec![cli_test("smoke", "$CLI_FORMULA ping", None)];
        let out = render_run_tests("o/t", "tool", None, "1.0.0", "tool", &tests);
        assert!(out.contains("_exit_smoke="), "must capture exit code");
        assert!(
            out.contains("[ \"$_exit_smoke\" -eq 0 ]"),
            "must check exit code is zero"
        );
        // Must NOT emit an expect_contains pattern comparison.
        assert!(!out.contains("*\"\"*"), "must not emit empty string comparison");
    }

    // --- render_readme ---

    #[test]
    fn render_readme_without_ffi_omits_ffi_row() {
        let tests = default_cli_tests("1.0.0");
        let out = render_readme("myorg/tap", "mytool", None, "1.0.0", &tests);
        assert!(out.contains("mytool"));
        assert!(!out.contains("Shared library"), "FFI row must be absent");
    }

    #[test]
    fn render_readme_with_ffi_includes_ffi_row() {
        let tests = default_cli_tests("1.0.0");
        let out = render_readme("myorg/tap", "mytool", Some("libmytool"), "1.0.0", &tests);
        assert!(out.contains("libmytool"));
        assert!(out.contains("Shared library"));
    }

    // --- ffi_smoke.c: domain-neutral version call ---

    #[test]
    fn render_ffi_smoke_c_calls_version_not_convert() {
        let out = render_ffi_smoke_c("mytool.h", "mytool", "libmytool");
        assert!(out.contains("mytool_version()"), "must call _version()");
        assert!(!out.contains("_convert("), "must NOT call _convert — domain-neutral");
        assert!(!out.contains("<h1>Hi</h1>"), "must NOT contain HTML test payload");
        // _version() returns `const char *`, not a Result struct — no HtmResult / error_code pattern.
        assert!(
            out.contains("const char *version = mytool_version()"),
            "must use const char * return, not a Result struct"
        );
        assert!(!out.contains("error_code"), "must NOT reference error_code");
        assert!(!out.contains("mytool_result_free"), "must NOT call result_free");
    }

    // --- render_run_tests version substitution ---

    #[test]
    fn render_run_tests_parameterizes_version_correctly() {
        let tests = default_cli_tests("1.9.0-rc.25");
        let out = render_run_tests("myorg/tap", "mytool", None, "1.9.0-rc.25", "mytool", &tests);
        // Version must be parameterized into the script.
        assert!(
            out.contains("VERSION='1.9.0-rc.25'"),
            "must set VERSION variable to the provided version"
        );
        assert!(
            out.contains("_expected_version='1.9.0-rc.25'"),
            "must render the resolved version as inert expectation data"
        );
        // The $CLI_FORMULA variable must be used to call the binary.
        assert!(
            out.contains("_command_version='$CLI_FORMULA --version'") && out.contains("eval \"$_command_version\""),
            "must use $CLI_FORMULA variable"
        );
    }

    #[test]
    fn render_run_tests_version_used_in_default_test() {
        let tests = default_cli_tests("2.0.0");
        let out = render_run_tests("myorg/tap", "mytool", None, "2.0.0", "mytool", &tests);
        // The default --version test should check for the parameterized VERSION.
        assert!(out.contains("VERSION='2.0.0'"));
        assert!(out.contains("if [[ \"$_output_version\" == *\"$_expected_version\"* ]]"));
    }

    #[test]
    fn default_version_expectation_matches_the_rendered_version_value() {
        let tests = default_cli_tests("1.9.0-rc.25");
        let out = render_run_tests("myorg/tap", "mytool", None, "1.9.0-rc.25", "mytool", &tests);
        let version = out.lines().find(|line| line.starts_with("VERSION=")).unwrap();
        let expected = out.lines().find(|line| line.starts_with("_expected_version=")).unwrap();
        let comparison = out
            .lines()
            .find(|line| line.starts_with("if [[ \"$_output_version\""))
            .and_then(|line| line.strip_suffix("; then"))
            .and_then(|line| line.strip_prefix("if "))
            .unwrap();
        let script = format!("{version}\n{expected}\n_output_version='mytool 1.9.0-rc.25'\n{comparison}");
        let status = crate::core::bash_command().args(["-c", &script]).status().unwrap();
        assert!(
            status.success(),
            "rendered default version comparison failed:\n{script}"
        );
    }

    #[test]
    fn custom_expected_text_remains_inert_during_comparison() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("expected-injection");
        let malicious = format!("$(touch {})", marker.display());
        let tests = vec![cli_test("hostile", "printf harmless", Some(&malicious))];
        let out = render_run_tests("myorg/tap", "mytool", None, "1.0.0", "mytool", &tests);
        let expected = out.lines().find(|line| line.starts_with("_expected_hostile=")).unwrap();
        let comparison = out
            .lines()
            .find(|line| line.starts_with("if [[ \"$_output_hostile\""))
            .and_then(|line| line.strip_suffix("; then"))
            .and_then(|line| line.strip_prefix("if "))
            .unwrap();
        let output = crate::core::config::shell::quote_word(&malicious);
        let script = format!("{expected}\n_output_hostile={output}\n{comparison}");
        let status = crate::core::bash_command().args(["-c", &script]).status().unwrap();
        assert!(status.success(), "literal expected text did not match:\n{script}");
        assert!(!marker.exists(), "custom expected text executed as shell syntax");
    }

    #[test]
    fn homebrew_config_values_are_emitted_as_inert_literals() {
        let malicious = "value'$(touch /tmp/alef-homebrew)'#{system('false')}";
        let brewfile = render_brewfile(malicious, malicious, Some(malicious));
        assert!(
            brewfile.contains(r"\#{system('false')}"),
            "Ruby interpolation markers must be escaped: {brewfile}"
        );

        let script = render_run_tests(malicious, malicious, Some(malicious), malicious, malicious, &[]);
        let status = crate::core::bash_command()
            .args(["-n", "-c", &script])
            .status()
            .expect("bash should parse generated script");
        assert!(status.success(), "generated shell must remain syntactically valid");
    }
}
