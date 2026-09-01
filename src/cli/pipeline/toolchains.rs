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

use crate::core::config::extras::Language;
use crate::core::config::tools::{ToolsConfig, required_tools_for_language};

use super::format::is_tool_available;

/// Hard-fail if any language in `languages` is missing a tool from its required toolchain.
///
/// Scoped to `languages` only -- a crate that never enables Ruby is never asked for `ruby`. See
/// [`required_tools_for_language`] for the per-language table.
pub fn enforce_required_toolchains(languages: &[Language], tools: &ToolsConfig) -> anyhow::Result<()> {
    enforce_required_toolchains_with(languages, tools, &is_tool_available)
}

/// Testable seam for [`enforce_required_toolchains`]: resolves each required tool through
/// `is_available` instead of PATH, the same seam `poly_lint_with` uses, so the missing-toolchain
/// bail is provable without depending on whether the host running the suite has the tool
/// installed. ~keep
pub(crate) fn enforce_required_toolchains_with(
    languages: &[Language],
    tools: &ToolsConfig,
    is_available: &dyn Fn(&str) -> bool,
) -> anyhow::Result<()> {
    for &lang in languages {
        for tool in required_tools_for_language(lang, tools) {
            if !is_available(&tool) {
                anyhow::bail!(
                    "{tool} not found on PATH; {lang} is enabled for this crate and requires it -- install \
                     {tool}, or remove {lang} from this crate's languages if it should not be enabled"
                );
            }
        }
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
        let error = enforce_required_toolchains_with(&[Language::Rust], &tools(), &|_tool| false)
            .expect_err("a missing cargo must fail, not report clean");
        assert!(error.to_string().contains("cargo"), "{error}");
    }

    #[test]
    fn bails_when_a_required_tool_is_missing_for_python() {
        let error = enforce_required_toolchains_with(&[Language::Python], &tools(), &|_tool| false)
            .expect_err("a missing python package manager must fail, not report clean");
        assert!(error.to_string().contains("uv"), "{error}");
    }

    #[test]
    fn bails_when_a_required_tool_is_missing_for_ruby() {
        let error = enforce_required_toolchains_with(&[Language::Ruby], &tools(), &|_tool| false)
            .expect_err("a missing ruby must fail, not report clean");
        assert!(error.to_string().contains("ruby"), "{error}");
    }

    /// ~keep The bare interpreter being present must NOT satisfy Ruby: `ruby` is on essentially
    /// every image, so a probe that stops there is the vacuous check this whole gate exists to
    /// remove. What the Ruby steps actually run is `ruby -S bundle`.
    #[test]
    fn bails_for_ruby_when_only_the_interpreter_is_present_and_bundler_is_not() {
        let error = enforce_required_toolchains_with(&[Language::Ruby], &tools(), &|tool| tool == "ruby")
            .expect_err("a present ruby with no bundler must still fail");
        assert!(error.to_string().contains("bundle"), "{error}");
    }

    #[test]
    fn bails_on_the_missing_cargo_edit_subcommand_for_rust() {
        // ~keep Only `cargo-upgrade` is absent; `cargo` itself resolves. Proves the check walks
        // every required tool for a language instead of stopping at the first hit.
        let error = enforce_required_toolchains_with(&[Language::Rust], &tools(), &|tool| tool == "cargo")
            .expect_err("a missing cargo-upgrade (cargo-edit) must fail even when cargo itself is present");
        assert!(error.to_string().contains("cargo-upgrade"), "{error}");
    }

    /// The converse of the bails-when-missing tests above: a language that is not in
    /// `languages` must never be probed for its tool, even when every probe would fail. This is
    /// the "a repo that does not enable Ruby must not need bundler" requirement.
    #[test]
    fn does_not_require_a_tool_for_a_language_that_is_not_enabled() {
        enforce_required_toolchains_with(&[Language::Python], &tools(), &|_tool| false)
            .expect_err("python is enabled, so its own probe must still bail");

        // Ruby is not in the enabled list at all: every probe returning false must still pass,
        // because the ruby toolchain is never even checked.
        enforce_required_toolchains_with(&[], &tools(), &|_tool| false)
            .expect("no languages enabled means no toolchain is required");

        // ~keep The empty-list case above is satisfied by any implementation that iterates
        // `languages`, including one that then probes an unrelated language's tools. This is the
        // case that actually encodes the requirement: Python IS enabled and its tool resolves,
        // Ruby is NOT enabled and none of its tools resolve, and the call must still succeed.
        enforce_required_toolchains_with(&[Language::Python], &tools(), &|tool| tool == "uv")
            .expect("a crate that does not enable ruby must not require ruby or bundler");
    }
}
