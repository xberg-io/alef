//! Hard toolchain preflight for enabled languages.
//!
//! `require_tool` (`core::config::tools`) used to be the entire enforcement mechanism for a
//! language's toolchain: it built a `command -v <tool>` precondition string that
//! `check_precondition` (`cli::pipeline::helpers`) treated as a skip switch on failure, so a
//! missing `uv`/`pnpm`/`cargo-edit` made `setup`/`build`/`test`/`update`/`clean` report success
//! having done nothing for that language. This module is the fix: for every language actually
//! enabled on a crate, it hard-fails before any per-command precondition is even evaluated,
//! following the same shape as `poly_lint_with` (`cli::pipeline::format`) -- a hard
//! `anyhow::bail!` plus an injectable probe seam so the failure is provable without depending on
//! whether the host running the suite happens to have the tool installed. ~keep
//!
//! alef 0.82.0 introduced a regression on top of that fix: [`enforce_required_toolchains`] asked
//! for the UNION of every tool any of `build`/`test`/`setup`/`update`/`clean` might ever need from
//! an enabled language, instead of the tools the COMMAND ACTUALLY INVOKING IT needs. `alef test
//! --lang rust` hard-failed over a missing `cargo-upgrade` -- a tool only `alef update`'s Rust arm
//! ever runs -- and `alef all` hard-failed over a missing `dart` even though `all` never shells out
//! to the `dart` binary itself (its only Dart-language work is `flutter_rust_bridge_codegen`,
//! which already tolerates a missing tool as a non-fatal skip because `VerifyFrbBridgeCoverage`
//! catches a resulting stale bridge structurally, see `core::backend`'s doc on that `RunCommand`).
//! [`ToolchainCommand`] and [`enforce_required_toolchains_for_all`] are the fix: the probe is
//! scoped to what the invoking command actually needs, not to the language's whole tool union.
//! ~keep

use crate::core::config::extras::Language;
use crate::core::config::tools::{ToolchainCommand, ToolsConfig, required_tools_for_language};

use super::format::is_tool_available;

/// Hard-fail if any language in `languages` is missing a tool that `command` actually needs from
/// its toolchain.
///
/// Scoped to `languages` only -- a crate that never enables Ruby is never asked for `ruby` -- and
/// to `command` -- `alef test` is never asked for `cargo-upgrade`, which only `alef update`'s Rust
/// arm runs. See [`required_tools_for_language`] for the per-command, per-language table.
pub fn enforce_required_toolchains(
    command: ToolchainCommand,
    languages: &[Language],
    tools: &ToolsConfig,
) -> anyhow::Result<()> {
    enforce_required_toolchains_with(command, languages, tools, &is_tool_available)
}

/// Testable seam for [`enforce_required_toolchains`]: resolves each required tool through
/// `is_available` instead of PATH, the same seam `poly_lint_with` uses, so the missing-toolchain
/// bail is provable without depending on whether the host running the suite has the tool
/// installed. ~keep
pub(crate) fn enforce_required_toolchains_with(
    command: ToolchainCommand,
    languages: &[Language],
    tools: &ToolsConfig,
    is_available: &dyn Fn(&str) -> bool,
) -> anyhow::Result<()> {
    for &lang in languages {
        for tool in required_tools_for_language(command, lang, tools) {
            bail_if_missing(&tool, lang, is_available)?;
        }
    }
    Ok(())
}

/// `alef all`'s own, much narrower toolchain preflight.
///
/// `all` never runs `build`/`test`/`setup`/`update`/`clean`'s pipeline functions -- it only
/// extracts, generates, formats, and (unconditionally) runs the cargo-based post-build steps in
/// `bin_cli::helpers::complete_generated_artifacts` (the FFI header refresh, and Swift's
/// generate-time `cargo check`). Both of those need `cargo`, gated on the languages that actually
/// trigger them (`Ffi`, `Swift`); no other language's `all`-time work shells out to an external
/// tool that isn't already independently tolerant of being missing (Dart's
/// `flutter_rust_bridge_codegen` RunCommand, the Dart lockfile relock -- see this module's doc).
/// Reusing [`required_tools_for_language`]'s `Build`/`Test`/... table here is exactly the
/// over-broad regression this module fixes: it would once again demand `dart`/`gradle`/`dotnet`
/// for a command that never invokes them. ~keep
pub fn enforce_required_toolchains_for_all(languages: &[Language]) -> anyhow::Result<()> {
    enforce_required_toolchains_for_all_with(languages, &is_tool_available)
}

/// Testable seam for [`enforce_required_toolchains_for_all`], mirroring
/// [`enforce_required_toolchains_with`]. ~keep
pub(crate) fn enforce_required_toolchains_for_all_with(
    languages: &[Language],
    is_available: &dyn Fn(&str) -> bool,
) -> anyhow::Result<()> {
    let needs_cargo = languages
        .iter()
        .any(|lang| matches!(lang, Language::Ffi | Language::Swift));
    if needs_cargo {
        bail_if_missing("cargo", Language::Ffi, is_available)?;
    }
    Ok(())
}

fn bail_if_missing(tool: &str, lang: Language, is_available: &dyn Fn(&str) -> bool) -> anyhow::Result<()> {
    if !is_available(tool) {
        anyhow::bail!(
            "{tool} not found on PATH; {lang} is enabled for this crate and requires it -- install \
             {tool}, or remove {lang} from this crate's languages if it should not be enabled"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> ToolsConfig {
        ToolsConfig::default()
    }

    /// `poly_lint_fails_loudly_when_poly_is_not_on_path`'s shape, generalised to every language:
    /// a missing required tool must bail, not report a clean run that did nothing.
    #[test]
    fn bails_when_a_required_tool_is_missing_for_rust() {
        let error =
            enforce_required_toolchains_with(ToolchainCommand::Build, &[Language::Rust], &tools(), &|_tool| false)
                .expect_err("a missing cargo must fail, not report clean");
        assert!(error.to_string().contains("cargo"), "{error}");
    }

    #[test]
    fn bails_when_a_required_tool_is_missing_for_python() {
        let error =
            enforce_required_toolchains_with(ToolchainCommand::Build, &[Language::Python], &tools(), &|_tool| false)
                .expect_err("a missing python package manager must fail, not report clean");
        assert!(error.to_string().contains("uv"), "{error}");
    }

    #[test]
    fn bails_when_a_required_tool_is_missing_for_ruby() {
        let error =
            enforce_required_toolchains_with(ToolchainCommand::Test, &[Language::Ruby], &tools(), &|_tool| false)
                .expect_err("a missing ruby must fail, not report clean");
        assert!(error.to_string().contains("ruby"), "{error}");
    }

    /// ~keep The bare interpreter being present must NOT satisfy Ruby: `ruby` is on essentially
    /// every image, so a probe that stops there is the vacuous check this whole gate exists to
    /// remove. What the Ruby steps actually run is `ruby -S bundle`.
    #[test]
    fn bails_for_ruby_when_only_the_interpreter_is_present_and_bundler_is_not() {
        let error = enforce_required_toolchains_with(ToolchainCommand::Test, &[Language::Ruby], &tools(), &|tool| {
            tool == "ruby"
        })
        .expect_err("a present ruby with no bundler must still fail");
        assert!(error.to_string().contains("bundle"), "{error}");
    }

    /// The regression this module fixes: `alef test --lang rust` must succeed with
    /// `cargo-upgrade` absent from PATH -- `test` never runs `cargo upgrade`, only `alef update`'s
    /// Rust arm does.
    #[test]
    fn test_does_not_require_cargo_upgrade_for_rust() {
        enforce_required_toolchains_with(ToolchainCommand::Test, &[Language::Rust], &tools(), &|tool| {
            tool == "cargo"
        })
        .expect("alef test never runs `cargo upgrade`; a missing cargo-upgrade must not fail it");
    }

    /// Same shape as `test`, for every other command that is not `update`.
    #[test]
    fn build_setup_and_clean_do_not_require_cargo_upgrade_for_rust() {
        for command in [
            ToolchainCommand::Build,
            ToolchainCommand::Setup,
            ToolchainCommand::Clean,
        ] {
            enforce_required_toolchains_with(command, &[Language::Rust], &tools(), &|tool| tool == "cargo")
                .unwrap_or_else(|error| panic!("{command:?} must not require cargo-upgrade: {error}"));
        }
    }

    /// The other direction of the fix: `alef update` must still hard-fail without `cargo-upgrade`
    /// -- weakening the gate back to a warning is the exact defect 0.82.0 exists to remove.
    #[test]
    fn update_still_hard_fails_without_cargo_upgrade_for_rust() {
        let error = enforce_required_toolchains_with(ToolchainCommand::Update, &[Language::Rust], &tools(), &|tool| {
            tool == "cargo"
        })
        .expect_err("alef update's Rust arm runs `cargo upgrade`; a missing cargo-upgrade must still fail it");
        assert!(error.to_string().contains("cargo-upgrade"), "{error}");
    }

    #[test]
    fn bails_on_the_missing_cargo_edit_subcommand_for_rust_update() {
        // ~keep Only `cargo-upgrade` is absent; `cargo` itself resolves. Proves the check walks
        // every required tool for a language instead of stopping at the first hit.
        let error = enforce_required_toolchains_with(ToolchainCommand::Update, &[Language::Rust], &tools(), &|tool| {
            tool == "cargo"
        })
        .expect_err("a missing cargo-upgrade (cargo-edit) must fail even when cargo itself is present");
        assert!(error.to_string().contains("cargo-upgrade"), "{error}");
    }

    /// The converse of the bails-when-missing tests above: a language that is not in
    /// `languages` must never be probed for its tool, even when every probe would fail. This is
    /// the "a repo that does not enable Ruby must not need bundler" requirement.
    #[test]
    fn does_not_require_a_tool_for_a_language_that_is_not_enabled() {
        enforce_required_toolchains_with(ToolchainCommand::Build, &[Language::Python], &tools(), &|_tool| false)
            .expect_err("python is enabled, so its own probe must still bail");

        // Ruby is not in the enabled list at all: every probe returning false must still pass,
        // because the ruby toolchain is never even checked.
        enforce_required_toolchains_with(ToolchainCommand::Build, &[], &tools(), &|_tool| false)
            .expect("no languages enabled means no toolchain is required");

        // ~keep The empty-list case above is satisfied by any implementation that iterates
        // `languages`, including one that then probes an unrelated language's tools. This is the
        // case that actually encodes the requirement: Python IS enabled and its tool resolves,
        // Ruby is NOT enabled and none of its tools resolve, and the call must still succeed.
        enforce_required_toolchains_with(ToolchainCommand::Build, &[Language::Python], &tools(), &|tool| {
            tool == "uv"
        })
        .expect("a crate that does not enable ruby must not require ruby or bundler");
    }

    /// The second symptom of the same over-broad-gate regression: `alef all` on a crate with
    /// `dart` enabled must not demand `dart` on PATH -- `all` never shells out to it directly (see
    /// this module's doc for why Dart's own tool-invoking steps already tolerate its absence).
    #[test]
    fn all_does_not_require_dart_even_when_enabled() {
        enforce_required_toolchains_for_all_with(&[Language::Dart], &|_tool| false)
            .expect("alef all never shells out to `dart` directly; it must not require it");
    }

    /// `alef all` DOES need `cargo` when `ffi` is enabled -- the FFI header refresh always runs a
    /// real `cargo build` (see `bin_cli::helpers::refresh_ffi_header`).
    #[test]
    fn all_requires_cargo_when_ffi_is_enabled() {
        let error = enforce_required_toolchains_for_all_with(&[Language::Ffi], &|_tool| false)
            .expect_err("a missing cargo must fail alef all when ffi is enabled");
        assert!(error.to_string().contains("cargo"), "{error}");
    }

    /// Same, for `swift`: `all`'s generate-time post-build downgrades to a `cargo check`, which
    /// still needs `cargo` (see `backends::swift::gen_bindings`'s `generate_post_build_config`
    /// override).
    #[test]
    fn all_requires_cargo_when_swift_is_enabled() {
        let error = enforce_required_toolchains_for_all_with(&[Language::Swift], &|_tool| false)
            .expect_err("a missing cargo must fail alef all when swift is enabled");
        assert!(error.to_string().contains("cargo"), "{error}");
    }

    /// A crate enabling neither `ffi` nor `swift` needs no toolchain at all for `alef all`.
    #[test]
    fn all_requires_nothing_for_a_crate_with_neither_ffi_nor_swift() {
        enforce_required_toolchains_for_all_with(&[Language::Python, Language::Node], &|_tool| false)
            .expect("alef all needs no external tool for a crate with neither ffi nor swift enabled");
    }
}
