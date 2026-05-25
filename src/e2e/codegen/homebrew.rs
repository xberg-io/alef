//! Homebrew test_app generator.
//!
//! Generates a registry-mode-only test_app at `test_apps/homebrew/` that
//! exercises both the CLI formula (`sample-markdown`) and the FFI formula
//! (`libsample-markdown`) from the sample_core Homebrew tap.
//!
//! Emits four files:
//!
//! - `Brewfile` — declares tap + formulae for `brew bundle install`.
//! - `run_tests.sh` — installs via Brewfile, checks CLI version, pipes HTML
//!   through the CLI binary, and compiles + runs `ffi_smoke.c`.
//! - `ffi_smoke.c` — minimal C program linking against the FFI formula.
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
        let ffi_formula = pkg
            .and_then(|p| p.ffi_formula.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("lib{cli_formula}"));
        let version = pkg
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        Ok(vec![
            GeneratedFile {
                path: output_base.join("Brewfile"),
                content: render_brewfile(&tap, &cli_formula, &ffi_formula),
                generated_header: false,
            },
            GeneratedFile {
                path: output_base.join("run_tests.sh"),
                content: render_run_tests(&tap, &cli_formula, &ffi_formula, &version),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("ffi_smoke.c"),
                content: render_ffi_smoke_c(),
                generated_header: true,
            },
            GeneratedFile {
                path: output_base.join("README.md"),
                content: render_readme(&tap, &cli_formula, &ffi_formula, &version),
                generated_header: false,
            },
        ])
    }

    fn language_name(&self) -> &'static str {
        "homebrew"
    }
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
fn render_brewfile(tap: &str, cli_formula: &str, ffi_formula: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Brewfile — managed by alef. DO NOT EDIT.");
    let _ = writeln!(out, "tap \"{tap}\"");
    let _ = writeln!(out, "brew \"{tap}/{cli_formula}\"");
    let _ = writeln!(out, "brew \"{tap}/{ffi_formula}\"");
    out
}

/// Render `run_tests.sh`.
fn render_run_tests(tap: &str, cli_formula: &str, ffi_formula: &str, version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# Tests the Homebrew CLI formula and FFI formula.");
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let _ = writeln!(out, "VERSION=\"{version}\"");
    let _ = writeln!(out, "TAP=\"{tap}\"");
    let _ = writeln!(out, "CLI_FORMULA=\"{cli_formula}\"");
    let _ = writeln!(out, "FFI_FORMULA=\"{ffi_formula}\"");
    // Fully-qualified names disambiguate when multiple taps export the same
    // formula short-name on a developer's machine (e.g. legacy + new tap).
    let _ = writeln!(out, "CLI_FORMULA_QUALIFIED=\"$TAP/$CLI_FORMULA\"");
    let _ = writeln!(out, "FFI_FORMULA_QUALIFIED=\"$TAP/$FFI_FORMULA\"");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "SCRIPT_DIR=\"$(cd \"$(dirname \"${{BASH_SOURCE[0]}}\")\" && pwd)\""
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "PASS=0");
    let _ = writeln!(out, "FAIL=0");
    let _ = writeln!(out);

    // Helper.
    let _ = writeln!(out, "pass() {{ echo \"PASS: $1\"; PASS=$((PASS + 1)); }}");
    let _ = writeln!(out, "fail() {{ echo \"FAIL: $1 — $2\" >&2; FAIL=$((FAIL + 1)); }}");
    let _ = writeln!(out);

    // Step 1: brew bundle.
    let _ = writeln!(out, "# Install formulae.");
    let _ = writeln!(out, "brew bundle install --file=\"$SCRIPT_DIR/Brewfile\"");
    let _ = writeln!(out);

    // Step 2: CLI version check.
    let _ = writeln!(out, "# Test: CLI version output contains VERSION.");
    let _ = writeln!(out, "cli_version=$(\"$CLI_FORMULA\" --version 2>&1 || true)");
    let _ = writeln!(out, "if [[ \"$cli_version\" == *\"$VERSION\"* ]]; then");
    let _ = writeln!(out, "  pass \"cli-version\"");
    let _ = writeln!(out, "else");
    let _ = writeln!(
        out,
        "  fail \"cli-version\" \"expected '$VERSION' in version output, got: $cli_version\""
    );
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);

    // Step 3: CLI conversion smoke test.
    let _ = writeln!(out, "# Test: CLI converts <h1>Hi</h1> to markdown containing '# Hi'.");
    let _ = writeln!(out, "cli_output=$(echo '<h1>Hi</h1>' | \"$CLI_FORMULA\" 2>&1 || true)");
    let _ = writeln!(out, "if [[ \"$cli_output\" == *\"# Hi\"* ]]; then");
    let _ = writeln!(out, "  pass \"cli-convert-h1\"");
    let _ = writeln!(out, "else");
    let _ = writeln!(
        out,
        "  fail \"cli-convert-h1\" \"expected '# Hi' in output, got: $cli_output\""
    );
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);

    // Step 4: FFI smoke via C compilation.
    let _ = writeln!(out, "# Test: FFI formula — compile and run ffi_smoke.c.");
    let _ = writeln!(out, "TMP_DIR=$(mktemp -d)");
    let _ = writeln!(out, "trap 'rm -rf \"$TMP_DIR\"' EXIT");
    let _ = writeln!(out);
    let _ = writeln!(out, "if command -v pkg-config &>/dev/null; then");
    let _ = writeln!(
        out,
        "  FFI_CFLAGS=$(pkg-config --cflags \"$FFI_FORMULA\" 2>/dev/null || true)"
    );
    let _ = writeln!(
        out,
        "  FFI_LIBS=$(pkg-config --libs \"$FFI_FORMULA\" 2>/dev/null || true)"
    );
    let _ = writeln!(out, "else");
    let _ = writeln!(out, "  # Fallback: use brew --prefix to locate headers and libs.");
    let _ = writeln!(
        out,
        "  FFI_PREFIX=$(brew --prefix \"$FFI_FORMULA_QUALIFIED\" 2>/dev/null || true)"
    );
    let _ = writeln!(out, "  FFI_CFLAGS=\"-I$FFI_PREFIX/include\"");
    let _ = writeln!(out, "  FFI_LIBS=\"-L$FFI_PREFIX/lib -lhtml_to_markdown\"");
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

    // Summary.
    let _ = writeln!(out, "echo \"\"");
    let _ = writeln!(out, "echo \"Results: $PASS passed, $FAIL failed\"");
    let _ = writeln!(out, "[ \"$FAIL\" -eq 0 ]");
    out
}

/// Render `ffi_smoke.c`.
fn render_ffi_smoke_c() -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#include <html_to_markdown_ffi.h>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "  const char *html = \"<h1>Hi</h1>\";");
    let _ = writeln!(
        out,
        "  HtmlToMarkdownResult result = html_to_markdown_convert(html, NULL);"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "  if (result.error_code != 0) {{");
    let _ = writeln!(
        out,
        "    fprintf(stderr, \"FAIL: html_to_markdown_convert returned error %d: %s\\n\","
    );
    let _ = writeln!(
        out,
        "            result.error_code, result.error_message ? result.error_message : \"\");"
    );
    let _ = writeln!(out, "    html_to_markdown_result_free(result);");
    let _ = writeln!(out, "    return 1;");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  if (!result.content || strstr(result.content, \"Hi\") == NULL) {{"
    );
    let _ = writeln!(
        out,
        "    fprintf(stderr, \"FAIL: expected 'Hi' in output, got: %s\\n\","
    );
    let _ = writeln!(out, "            result.content ? result.content : \"(null)\");");
    let _ = writeln!(out, "    html_to_markdown_result_free(result);");
    let _ = writeln!(out, "    return 1;");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  printf(\"PASS: html_to_markdown_convert returned: %s\\n\", result.content);"
    );
    let _ = writeln!(out, "  html_to_markdown_result_free(result);");
    let _ = writeln!(out, "  return 0;");
    let _ = writeln!(out, "}}");
    out
}

/// Render `README.md`.
fn render_readme(tap: &str, cli_formula: &str, ffi_formula: &str, version: &str) -> String {
    format!(
        "# homebrew test_app\n\n\
         Exercises the configured Homebrew formulae from tap `{tap}` at version `{version}`.\n\n\
         | Formula | Purpose |\n\
         |---------|--------|\n\
         | `{cli_formula}` | CLI binary: converts HTML from stdin to Markdown on stdout |\n\
         | `{ffi_formula}` | Shared library: C FFI for embedding in other languages |\n\n\
         ## Running\n\n\
         ```bash\n\
         bash run_tests.sh\n\
         ```\n\n\
         ## What it tests\n\n\
         1. `brew bundle install` succeeds (tap + both formulae install without error).\n\
         2. `{cli_formula} --version` output contains `{version}`.\n\
         3. `echo '<h1>Hi</h1>' | {cli_formula}` produces output containing `# Hi`.\n\
         4. `ffi_smoke.c` compiles against `{ffi_formula}` (via `pkg-config`) and the\n\
            compiled binary converts `<h1>Hi</h1>` successfully.\n"
    )
}

/// Emit a test backend stub (not applicable for homebrew).
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    TestBackendEmission::unimplemented("homebrew")
}
