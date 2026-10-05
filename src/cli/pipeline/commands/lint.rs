use crate::cli::pipeline::format::{converge_full_regen_formatting, format_language_residual, poly_lint};
use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::Path;

/// Run the same converging whole-tree formatting pass `alef all` uses (see
/// `converge_full_regen_formatting`) on generated output.
///
/// This used to run a bespoke single `poly fmt --fix` pass plus the old per-language
/// `cargo sort` residual list instead of delegating here -- a second, independently
/// maintained formatting implementation that inevitably drifted from the one `alef all`
/// uses. `converge_full_regen_formatting` folds in a workspace-wide `cargo fmt --all`,
/// a workspace-wide `cargo sort -n -w` (covering crates the old per-language residual
/// list simply had no entry for -- python, node, php, swift, dart, ...), `mix format`
/// for Elixir's `.ex`/`.exs` source (the old path never ran `mix` at all), and loops
/// `poly fmt --fix`/`--check` to a fixed point because some poly-bundled engines are
/// not single-pass idempotent on freshly generated output. That last point is exactly
/// why the old single-pass `alef fmt` rewrote `packages/dart/.../frb_generated.dart`
/// to an incorrect intermediate form (relative imports, two dropped `dart:core`
/// imports) that a following `alef all` then reverted -- `alef fmt` and `alef all`
/// must agree on the canonical form of the same file, and the converging pass is the
/// one CI and the ownership guard's hash-stamping already treat as authoritative
/// (alef #126). ~keep
pub fn fmt(config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<()> {
    converge_full_regen_formatting(base_dir);
    if config.targets(Language::Dart) {
        format_language_residual(config, Language::Dart, base_dir);
    }
    Ok(())
}

/// Run `poly lint` on generated output. Propagates failure.
pub fn lint(_config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<()> {
    poly_lint(base_dir)
}

/// Run the same converging whole-tree formatting pass as [`fmt`], as a post-generation
/// best-effort pass. Never propagates failure (`converge_full_regen_formatting` is
/// itself best-effort throughout).
pub fn fmt_post_generate(config: &ResolvedCrateConfig, base_dir: &Path) {
    converge_full_regen_formatting(base_dir);
    if config.targets(Language::Dart) {
        format_language_residual(config, Language::Dart, base_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::pipeline::format::is_tool_available;
    use crate::core::config::NewAlefConfig;
    use crate::test_support::tool_available_with_stable_path;

    fn write_minimal_mix_project(elixir_dir: &Path, content: &str) {
        std::fs::create_dir_all(elixir_dir.join("lib")).unwrap();
        std::fs::write(
            elixir_dir.join("mix.exs"),
            "defmodule Sample.MixProject do\n  use Mix.Project\n\n  def project do\n    [app: :sample, \
             version: \"0.1.0\", elixir: \"~> 1.14\"]\n  end\nend\n",
        )
        .unwrap();
        std::fs::write(
            elixir_dir.join(".formatter.exs"),
            "[inputs: [\"mix.exs\", \"lib/**/*.{ex,exs}\"]]\n",
        )
        .unwrap();
        std::fs::write(elixir_dir.join("lib/sample.ex"), content).unwrap();
    }

    #[test]
    fn fmt_runs_dart_format_on_already_stamped_owned_files() {
        let Some(_path) = tool_available_with_stable_path("dart") else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let dart_dir = dir.path().join("packages/dart/lib");
        std::fs::create_dir_all(&dart_dir).expect("create Dart package");
        let dart_file = dart_dir.join("sample.dart");
        std::fs::write(
            &dart_file,
            "// alef:hash:0000000000000000000000000000000000000000000000000000000000000000\nvoid main(){print('ok');}\n",
        )
        .expect("write Dart source");
        let parsed: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["dart"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
"#,
        )
        .expect("valid config");
        let config = parsed.resolve().expect("resolved config").remove(0);

        fmt(&config, dir.path()).expect("fmt succeeds");

        let formatted = std::fs::read_to_string(dart_file).expect("read formatted Dart source");
        assert!(
            formatted.contains("void main() {\n  print('ok');\n}"),
            "the native Dart formatter must not skip stamped owned files:\n{formatted}"
        );
    }

    /// Regression for alef #126: `alef fmt` used to run a bespoke `poly_fmt` +
    /// `run_cargo_sort_residuals` pass that never invoked `mix format` at all --
    /// `.ex`/`.exs` source is excluded from poly's own pass (see
    /// `format::owner::POLY_EXCLUDE_GLOBS`) and `mix` is the sole formatter for it, so that old
    /// path left every `alef fmt`-only run's Elixir output completely unformatted. `fmt`
    /// must now reach the same converged state `alef all` does, which includes running
    /// `mix format`.
    #[test]
    fn fmt_runs_mix_format_the_same_way_alef_all_does() {
        let Some(_path) = tool_available_with_stable_path("mix") else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let elixir_dir = base.join("packages/elixir");
        write_minimal_mix_project(&elixir_dir, "defmodule Sample do\n  def noop, do:    :ok\nend\n");

        fmt(&ResolvedCrateConfig::default(), base).expect("fmt must succeed");

        assert_eq!(
            std::fs::read_to_string(elixir_dir.join("lib/sample.ex")).unwrap(),
            "defmodule Sample do\n  def noop, do: :ok\nend\n",
            "`alef fmt` must run `mix format` on generated Elixir source, the same way \
             `alef all` does"
        );
    }

    /// `alef fmt` must leave the tree workspace-sorted and `poly fmt --check`-clean --
    /// the same state `alef all` converges to -- so that running `alef fmt` then `alef
    /// all` (or the reverse) is a no-op in either order instead of each one undoing the
    /// other's idea of "formatted". This crate uses a `-py` suffix (standing in for
    /// python/node/php/swift/dart/... -- any language with no per-language cargo-sort
    /// residual) specifically because `cargo_sort_residuals`'s fixed step list already
    /// happened to include a workspace-wide `cargo sort -n -w` (via its unconditional
    /// `Language::Ffi` arm) even under the old implementation, so this assertion alone
    /// does not discriminate old from new -- `fmt_runs_mix_format_the_same_way_alef_all_does`
    /// above is the dynamic regression proof for #126. This test instead pins the
    /// property the fix is *for*: `fmt` must delegate to the identical
    /// `converge_full_regen_formatting` function `alef all` uses, not a parallel
    /// reimplementation that happens to agree today and can silently drift tomorrow. ~keep
    #[test]
    fn fmt_converges_to_the_same_workspace_sorted_poly_clean_state_as_alef_all() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        std::fs::write(
            base.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/pkg-py\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        let crate_dir = base.join("crates/pkg-py");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"pkg-py\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nserde = \"1\"\nanyhow = \"1\"\n",
        )
        .unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), "pub fn noop() {}\n").unwrap();

        fmt(&ResolvedCrateConfig::default(), base).expect("fmt must succeed");

        if is_tool_available("cargo-sort") {
            let toml = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
            let anyhow_pos = toml.find("anyhow").expect("anyhow present");
            let serde_pos = toml.find("serde").expect("serde present");
            assert!(
                anyhow_pos < serde_pos,
                "`alef fmt` must workspace-sort a crate with no per-language cargo-sort \
                 residual, got: {toml}"
            );
        }
        if is_tool_available("poly") {
            let check = std::process::Command::new("poly")
                .args(["fmt", "--check", &base.to_string_lossy()])
                .current_dir(base)
                .status()
                .expect("run poly fmt --check");
            assert!(
                check.success(),
                "the tree `alef fmt` leaves behind must already be poly-fmt-clean"
            );
        }
    }

    /// Structural pin for alef #126: both `fmt` (the `alef fmt` command) and
    /// `fmt_post_generate` must call `converge_full_regen_formatting` -- the exact
    /// function `alef all` calls via `format_generated(..., None)` -- rather than a
    /// second, independently maintained implementation that can silently drift from it
    /// again. A source-string check because the dynamic tests above can only prove
    /// behavioral agreement for the specific fixtures they construct; this proves the
    /// two commands cannot help but agree, by construction, for every case.
    #[test]
    fn fmt_and_fmt_post_generate_both_delegate_to_the_shared_converging_pass() {
        let source = include_str!("lint.rs");
        let fmt_start = source.find("pub fn fmt(").expect("fmt function");
        let fmt_post_start = source
            .find("pub fn fmt_post_generate(")
            .expect("fmt_post_generate function");
        let tests_mod_start = source.find("#[cfg(test)]").expect("test module marker");

        let fmt_body = &source[fmt_start..fmt_post_start];
        let fmt_post_body = &source[fmt_post_start..tests_mod_start];

        assert!(
            fmt_body.contains("converge_full_regen_formatting(base_dir)"),
            "`fmt` must delegate to `converge_full_regen_formatting`, not a bespoke pass"
        );
        assert!(
            fmt_post_body.contains("converge_full_regen_formatting(base_dir)"),
            "`fmt_post_generate` must delegate to `converge_full_regen_formatting`, not a \
             bespoke pass"
        );
    }
}
