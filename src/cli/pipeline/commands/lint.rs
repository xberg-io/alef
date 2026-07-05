use crate::cli::pipeline::format::{poly_fmt, poly_lint, run_cargo_sort_residuals};
use crate::core::config::ResolvedCrateConfig;
use std::path::Path;

/// Run `poly fmt --fix` and all cargo-sort residuals on generated output.
pub fn fmt(config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<()> {
    poly_fmt(base_dir);
    run_cargo_sort_residuals(config, base_dir);
    Ok(())
}

/// Run `poly lint` on generated output. Propagates failure.
pub fn lint(_config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<()> {
    poly_lint(base_dir)
}

/// Run `poly fmt --fix` and all cargo-sort residuals as a post-generation
/// best-effort pass. Never propagates failure.
pub fn fmt_post_generate(config: &ResolvedCrateConfig, base_dir: &Path) {
    poly_fmt(base_dir);
    run_cargo_sort_residuals(config, base_dir);
}
