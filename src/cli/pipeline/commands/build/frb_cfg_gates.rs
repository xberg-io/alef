//! Orchestration for [`crate::core::backend::PostBuildStep::CarryFrbCfgGates`]: reads the FRB
//! facade's `lib.rs` and the flutter_rust_bridge-generated `frb_generated.rs` off disk, carries
//! any `#[cfg(...)]` gates from the former into the latter, and writes back the canonical form.
//!
//! Split out of `build.rs` (already over this repo's 1,000-line cap) rather than left inline —
//! same rationale as `frb_bridge_coverage`. [`canonical_frb_generated`] is exported (not just
//! `pub(super)`) because `alef verify` needs the identical computation to detect drift this
//! step would silently fix on the next build: see alef #179.

use anyhow::Context as _;
use std::path::Path;

/// The canonical on-disk form of `frb_generated_source`, given the current `lib_rs_source`.
///
/// `flutter_rust_bridge_codegen` writes `frb_generated_source` raw and unformatted, using
/// whatever import ordering its own internal collections happened to produce that run. `alef
/// build` regenerates this file through this exact function and nothing else; `alef generate`
/// regenerates it the same way and then *additionally* runs the separate `poly fmt`-based
/// formatting pass (`pipeline::format_generated_reporting`) over the whole package tree. Running
/// [`crate::cli::pipeline::normalize_content`] here too — the same normalization the guarded
/// `write_files` path hashes against — is what makes those two alef commands agree on the
/// committed bytes for identical input instead of disagreeing (alef #179): without it, `alef
/// build` left the tool's raw ordering behind while `alef generate` normalized it away. ~keep
///
/// `project_root` bounds `normalize_content`'s rustfmt-config/edition lookups (alef #465) --
/// both of this function's callers (`run`, below, and `bin_cli::core_commands::verify`'s
/// `frb_generated_drift`) must pass the SAME root, or `alef build` and `alef verify` would
/// disagree about which `rustfmt.toml` applies to this file. ~keep
pub(crate) fn canonical_frb_generated(
    lib_rs_source: &str,
    frb_generated_source: &str,
    frb_generated_path: &Path,
    project_root: &Path,
) -> String {
    let gated = crate::backends::dart::carry_lib_rs_cfg_gates_into_frb_generated(lib_rs_source, frb_generated_source);
    crate::cli::pipeline::normalize_content(project_root, frb_generated_path, &gated)
}

/// Read `source_file` (the FRB facade's `lib.rs`) and `target_file` (`frb_generated.rs`), carry
/// any cfg gates across, normalize the result, and write it back if anything changed.
///
/// A no-op when either file does not exist yet — mirrors every other `PostBuildStep` handler in
/// `run_post_build`, which treats "nothing to check yet" as fine rather than an error.
pub(super) fn run(source_file: &Path, target_file: &Path, project_root: &Path) -> anyhow::Result<()> {
    if !source_file.exists() || !target_file.exists() {
        tracing::debug!(
            "CarryFrbCfgGates source or target not found: {} / {}",
            source_file.display(),
            target_file.display()
        );
        return Ok(());
    }

    let source_content = std::fs::read_to_string(source_file)
        .with_context(|| format!("failed to read cfg-gate source {}", source_file.display()))?;
    let target_content = std::fs::read_to_string(target_file)
        .with_context(|| format!("failed to read cfg-gate target {}", target_file.display()))?;

    let canonical = canonical_frb_generated(&source_content, &target_content, target_file, project_root);
    if canonical != target_content {
        std::fs::write(target_file, &canonical)
            .with_context(|| format!("failed to write cfg-gated file {}", target_file.display()))?;
        tracing::info!(
            "Carried #[cfg] gates from {} into {} and normalized its formatting",
            source_file.display(),
            target_file.display()
        );
    } else {
        tracing::debug!("CarryFrbCfgGates {}: no changes needed", target_file.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// alef #179: `alef build`'s `CarryFrbCfgGates` step used to write
    /// `flutter_rust_bridge_codegen`'s raw output straight to disk, while `alef generate` ran a
    /// separate `poly fmt` pass over the same file afterward. Two alef commands regenerating
    /// byte-identical input then disagreed on the committed bytes, purely because one path
    /// formatted and the other did not -- the file `lib.rs` here has zero `#[cfg(...)]`-gated
    /// functions, so `carry_lib_rs_cfg_gates_into_frb_generated` alone is a no-op on both inputs
    /// and would return each one back verbatim; only the added normalization pass can converge
    /// them. This feeds the real generator (`rustfmt`, via `normalize_content`) two
    /// textually-different-but-semantically-equal raw renderings of the same `use` list -- not a
    /// hand-sorted assertion -- and requires it to converge them to identical bytes.
    ///
    /// Skips (rather than failing) when `rustfmt` is not on `PATH`: `format_rust_content` is
    /// best-effort and returns its input unchanged when the tool is absent, which would make this
    /// assertion fail for an unrelated, environmental reason. Matches
    /// `generate::tests::test_normalize_content_strips_trailing_whitespace_when_rustfmt_fails`'s
    /// own tolerance for a missing toolchain. ~keep
    #[test]
    fn canonical_frb_generated_converges_raw_and_formatted_import_order() {
        // No `CWD_LOCK` here (alef #465): `canonical_frb_generated` now takes an explicit
        // `project_root` instead of `format_rust_content` resolving rustfmt's config against
        // `std::env::current_dir()`, so a sibling test's `test_support::CwdGuard` chdir can no
        // longer race this one.
        if !crate::cli::pipeline::is_tool_available("rustfmt") {
            return;
        }

        let project_root = tempfile::tempdir().expect("tempdir");
        let lib_rs = "pub fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n";
        let raw_from_tool = "use flutter_rust_bridge::for_generated::{transform_result_dco, Lifetimeable, Lockable};\n\
             fn wire__crate__add_impl() {}\n";
        let already_formatted = "use flutter_rust_bridge::for_generated::{Lifetimeable, Lockable, transform_result_dco};\n\
             fn wire__crate__add_impl() {}\n";
        assert_ne!(
            raw_from_tool, already_formatted,
            "fixture setup: these must start out textually different for the test to mean anything"
        );

        let path = Path::new("frb_generated.rs");
        let from_raw = canonical_frb_generated(lib_rs, raw_from_tool, path, project_root.path());
        let from_formatted = canonical_frb_generated(lib_rs, already_formatted, path, project_root.path());

        assert_eq!(
            from_raw, from_formatted,
            "canonical_frb_generated must converge regardless of which alef command last wrote \
             the raw file -- this is what a repeated `alef build`/`alef generate` regeneration on \
             unchanged input must never disagree on"
        );
    }

    /// Running the same canonicalization twice on its own already-canonical output must be a
    /// no-op (idempotent) -- the property `alef verify`'s frb-gate-drift check relies on to ever
    /// reach a clean state instead of flagging every single run as drifted.
    #[test]
    fn canonical_frb_generated_is_idempotent() {
        // No `CWD_LOCK` here either (alef #465) -- see the comment on
        // `canonical_frb_generated_converges_raw_and_formatted_import_order` above.
        if !crate::cli::pipeline::is_tool_available("rustfmt") {
            return;
        }

        let project_root = tempfile::tempdir().expect("tempdir");
        let lib_rs = "pub fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n";
        let raw = "use flutter_rust_bridge::for_generated::{transform_result_dco, Lifetimeable, Lockable};\n\
                   fn wire__crate__add_impl() {}\n";
        let path = Path::new("frb_generated.rs");

        let once = canonical_frb_generated(lib_rs, raw, path, project_root.path());
        let twice = canonical_frb_generated(lib_rs, &once, path, project_root.path());

        assert_eq!(
            once, twice,
            "canonicalizing already-canonical content must be a fixed point"
        );
    }
}
