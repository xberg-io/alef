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
//! expect_contains = "$VERSION"
//! ```

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::HomebrewCliTest;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::{E2eCodegen, TestBackendEmission};
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
            _ => default_cli_tests(),
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
}

/// Default CLI tests when none are configured: a single `--version` check.
fn default_cli_tests() -> Vec<HomebrewCliTest> {
    vec![HomebrewCliTest {
        name: "version".to_string(),
        command: "$CLI_FORMULA --version".to_string(),
        expect_contains: Some("$VERSION".to_string()),
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
    let _ = writeln!(out, "tap \"{tap}\"");
    let _ = writeln!(out, "brew \"{tap}/{cli_formula}\"");
    if let Some(ffi) = ffi_formula {
        let _ = writeln!(out, "brew \"{tap}/{ffi}\"");
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
    let _ = writeln!(out, "VERSION=\"{version}\"");
    let _ = writeln!(out, "TAP=\"{tap}\"");
    let _ = writeln!(out, "CLI_FORMULA=\"{cli_formula}\"");
    if let Some(ffi) = ffi_formula {
        let _ = writeln!(out, "FFI_FORMULA=\"{ffi}\"");
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
    let _ = writeln!(out, "# Install formulae.");
    let _ = writeln!(out, "brew bundle install --file=\"$SCRIPT_DIR/Brewfile\"");
    let _ = writeln!(out);

    // Emit each CLI test.
    for test in cli_tests {
        let _ = writeln!(out, "# Test: {}.", test.name);
        let _ = writeln!(
            out,
            "_output_{}=$(eval \"{}\" 2>&1 || true)",
            sanitize_var_name(&test.name),
            test.command
        );
        if let Some(ref expected) = test.expect_contains {
            let _ = writeln!(
                out,
                "if [[ \"$_output_{}\" == *\"{}\"* ]]; then",
                sanitize_var_name(&test.name),
                expected
            );
            let _ = writeln!(out, "  pass \"{}\"", test.name);
            let _ = writeln!(out, "else");
            let _ = writeln!(
                out,
                "  fail \"{}\" \"expected '{}' in output, got: $_output_{}\"",
                test.name,
                expected,
                sanitize_var_name(&test.name)
            );
            let _ = writeln!(out, "fi");
        } else {
            // No expected substring — just require zero exit code.
            let _ = writeln!(
                out,
                "_exit_{}=$(eval \"{}\" >/dev/null 2>&1; echo $?)",
                sanitize_var_name(&test.name),
                test.command
            );
            let _ = writeln!(out, "if [ \"$_exit_{}\" -eq 0 ]; then", sanitize_var_name(&test.name));
            let _ = writeln!(out, "  pass \"{}\"", test.name);
            let _ = writeln!(out, "else");
            let _ = writeln!(
                out,
                "  fail \"{}\" \"command exited with code $_exit_{}\"",
                test.name,
                sanitize_var_name(&test.name)
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
        let _ = writeln!(out, "  FFI_LIBS=\"-L$FFI_PREFIX/lib -l{ffi_lib_name}\"");
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
/// The smoke test calls a `{ffi_prefix}_version()` function that every FFI
/// library is expected to expose, validating that the formula is loadable and
/// the symbol is reachable without assuming any domain-specific API.
fn render_ffi_smoke_c(ffi_header: &str, ffi_prefix: &str, _ffi_formula: &str) -> String {
    let mut out = String::new();
    let result_type = format!("{}Result", ffi_prefix.to_upper_camel_case());
    let free_fn = format!("{ffi_prefix}_result_free");
    let version_fn = format!("{ffi_prefix}_version");
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#include <{ffi_header}>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "  {result_type} result = {version_fn}();");
    let _ = writeln!(out);
    let _ = writeln!(out, "  if (result.error_code != 0) {{");
    let _ = writeln!(
        out,
        "    fprintf(stderr, \"FAIL: {version_fn} returned error %d: %s\\n\","
    );
    let _ = writeln!(
        out,
        "            result.error_code, result.error_message ? result.error_message : \"\");"
    );
    let _ = writeln!(out, "    {free_fn}(result);");
    let _ = writeln!(out, "    return 1;");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  if (!result.content || strlen(result.content) == 0) {{");
    let _ = writeln!(
        out,
        "    fprintf(stderr, \"FAIL: expected non-empty version string\\n\");"
    );
    let _ = writeln!(out, "    {free_fn}(result);");
    let _ = writeln!(out, "    return 1;");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  printf(\"PASS: {version_fn} returned: %s\\n\", result.content);");
    let _ = writeln!(out, "  {free_fn}(result);");
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
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    TestBackendEmission::unimplemented("homebrew")
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
        let tests = default_cli_tests();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "version");
        assert_eq!(tests[0].command, "$CLI_FORMULA --version");
        assert_eq!(tests[0].expect_contains.as_deref(), Some("$VERSION"));
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
        assert!(out.contains("tap \"myorg/tap\""));
        assert!(out.contains("brew \"myorg/tap/mytool\""));
        assert!(!out.contains("libmytool"), "must not emit a default lib formula");
    }

    #[test]
    fn render_brewfile_with_ffi_includes_ffi_line() {
        let out = render_brewfile("myorg/tap", "mytool", Some("libmytool"));
        assert!(out.contains("brew \"myorg/tap/mytool\""));
        assert!(out.contains("brew \"myorg/tap/libmytool\""));
    }

    // --- render_run_tests without FFI ---

    #[test]
    fn render_run_tests_no_ffi_omits_ffi_section() {
        let tests = default_cli_tests();
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
        let tests = default_cli_tests();
        let out = render_run_tests("myorg/tap", "mytool", None, "1.2.3", "mytool", &tests);
        assert!(out.contains("VERSION=\"1.2.3\""));
        assert!(out.contains("CLI_FORMULA=\"mytool\""));
        // default --version check uses $VERSION as expected substring
        assert!(out.contains("$VERSION"));
    }

    // --- render_run_tests with FFI ---

    #[test]
    fn render_run_tests_with_ffi_includes_ffi_section() {
        let tests = default_cli_tests();
        let out = render_run_tests("myorg/tap", "mytool", Some("libmytool"), "1.0.0", "mytool", &tests);
        assert!(out.contains("FFI_FORMULA=\"libmytool\""));
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
        let tests = default_cli_tests();
        let out = render_readme("myorg/tap", "mytool", None, "1.0.0", &tests);
        assert!(out.contains("mytool"));
        assert!(!out.contains("Shared library"), "FFI row must be absent");
    }

    #[test]
    fn render_readme_with_ffi_includes_ffi_row() {
        let tests = default_cli_tests();
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
    }
}
