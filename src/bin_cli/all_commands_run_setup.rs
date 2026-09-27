//! Pre-flight and cross-cutting helper functions for `alef all`'s `handle`, split out of
//! `all_commands.rs` for the file-modularization cap: version resolution/sync before the run,
//! deferred-formatting reporting, snippet-validation refusal correlation, and the
//! create-once overwrite policy.

use anyhow::{Context as _, Result};
use std::path::PathBuf;

use crate::cli::pipeline;

/// Surface formatting steps that did not run: registry-mode dependency resolution deferred
/// to a post-publish pass, and formatters whose executable is absent.
///
/// Deliberately not an error, and deliberately routed through the same reporter the
/// standalone stage commands use (`e2e::format::warn_deferred`) rather than classifying the
/// entries a second time here -- this copy used to blame every entry, including a missing
/// formatter, on an unpublished version. Registry-mode manifests pin the version the current
/// run produces, so those steps cannot succeed until that version is published; failing here
/// would mean every release run fails on a precondition required to be false at that moment.
/// Local-mode e2e, which is what actually gates correctness, still hard-fails on any
/// formatter error. ~keep
pub(crate) fn report_deferred_formatting(crate_name: &str, deferred: &[crate::e2e::format::DeferredFormatting]) {
    crate::e2e::format::warn_deferred_for_crate(crate_name, deferred);
}

/// Paths this run's ownership guard refused to write that fall inside the crate's
/// configured `docs.snippets` roots (`dirs`, `inline_dirs`, minus `exclude`).
///
/// Mirrors the directory-level inclusion/exclusion `docs::build_snippet_context` applies
/// before `discover_snippets` walks disk -- not a re-implementation of snippet discovery
/// itself, just enough to tell whether a refusal earlier in this run landed inside the
/// tree that `generate_docs_stage`'s snippet validation later reads back off disk. A
/// non-empty result means that validation -- pass or fail -- was graded against bytes
/// this run never wrote, which is invisible unless something correlates the two. ~keep
pub(crate) fn refused_snippet_dir_paths(
    refused_paths: &std::collections::BTreeSet<PathBuf>,
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<PathBuf> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Vec::new();
    };
    let snippet_dirs: Vec<PathBuf> = snippet_cfg
        .dirs
        .iter()
        .chain(&snippet_cfg.inline_dirs)
        .map(|dir| base_dir.join(dir))
        .collect();
    if snippet_dirs.is_empty() {
        return Vec::new();
    }
    let excluded: Vec<PathBuf> = snippet_cfg.exclude.iter().map(|dir| base_dir.join(dir)).collect();
    refused_paths
        .iter()
        .filter(|path| snippet_dirs.iter().any(|dir| path.starts_with(dir)))
        .filter(|path| !excluded.iter().any(|prefix| path.starts_with(prefix)))
        .cloned()
        .collect()
}

pub(crate) fn sync_registry_versions_before_all(
    config_path: &std::path::Path,
    configs: &[&crate::core::config::ResolvedCrateConfig],
) -> Result<bool> {
    let mut versions = std::collections::BTreeSet::new();
    for config in configs {
        let version = config.resolved_version().with_context(|| {
            format!(
                "could not resolve version for crate `{}` from {}",
                config.name, config.version_from
            )
        })?;
        versions.insert(version);
    }
    anyhow::ensure!(
        versions.len() <= 1,
        "alef all cannot synchronize one registry config from multiple crate versions: {}",
        versions.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let Some(version) = versions.into_iter().next() else {
        return Ok(false);
    };
    pipeline::sync_registry_package_versions(config_path, &version)
}

/// The `overwrite` argument `alef all`'s create-once-bearing write stages hand to
/// [`pipeline::write_scaffold_files_report`].
///
/// `clean` is taken and deliberately does not participate. Until this function existed the
/// scaffold and docs stages passed `clean` straight through, which made one flag mean two
/// unrelated things: "ignore cached IR" and "disable the create-only branch that leaves a
/// pre-existing unmarked file alone". Only the second one destroys work — a hand-grown
/// `composer.json` becomes alef's placeholder — and nothing about wanting a cache-cold rerun
/// implies wanting that. The parameter stays in the signature so the separation is an
/// executable fact with a test behind it rather than an absence a later edit can silently
/// undo by reaching for the `clean` that is already in scope at both call sites. ~keep
pub(crate) fn create_once_overwrite(clean: bool, clobber_create_once_seeds: bool) -> bool {
    let _ = clean;
    clobber_create_once_seeds
}

/// Last run's per-language binding ownership, read from the dedicated `all-bindings-{lang}
/// -ownership` stage manifest -- extracted out of `handle`'s main loop purely for the
/// file-modularization cap on `all_commands.rs` (alef #465), no behaviour change.
///
/// See the call site in `handle` for why this has to be read from that specific manifest and
/// not from `<lang>.manifest`: `pipeline::generate` unconditionally overwrites the latter for
/// every language it regenerates, so a "previous" read of it after that call would already be
/// reading this run's own output. ~keep
pub(crate) fn previous_binding_ownership(
    languages: &[crate::core::config::Language],
    crate_name: &str,
) -> std::collections::HashMap<crate::core::config::Language, Vec<PathBuf>> {
    languages
        .iter()
        .map(|language| {
            (
                *language,
                crate::cli::cache::read_stage_paths(crate_name, &format!("all-bindings-{language}-ownership")),
            )
        })
        .collect()
}
