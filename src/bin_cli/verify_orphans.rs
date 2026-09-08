//! Orphan detection for `alef verify`.
//!
//! A file a backend used to emit and then quietly stopped emitting sits on disk forever: nothing
//! `alef verify` ran before this module existed ever diffed the disk-side alef-marked file set
//! against what the current run's backends would actually produce, so a dropped emit was
//! invisible across every release until a human happened to notice the stale file by hand (see
//! the Java `NodeContext.java`/`HtmlVisitor.java`/`VisitorBridge.java` case this module exists
//! to catch). [`find_orphaned_generated_files`] closes that gap by comparing
//! [`super::helpers::collect_alef_hashes`]'s disk walk (the same one [`super::helpers::verify_walk`]
//! and [`super::helpers::verify_walk_multi`] already use for staleness) against the expected
//! output-path set [`super::helpers::find_missing_and_frozen_generated_files`] already computes
//! from a full in-memory regeneration.
//!
//! This module only ever reports. Deleting an orphan automatically is unrecoverable if the
//! detection is wrong even once, and it can be wrong for reasons that have nothing to do with
//! the file being stale: a create-once seed a backend emits only when absent (`rust-toolchain.toml`,
//! the wasm `.cargo/config.toml` -- see `src/scaffold/mod.rs`'s `rust_toolchain_file` and
//! `src/scaffold/languages/wasm.rs`'s `wasm_cargo_config_file`, both gated on the path already
//! existing), or a tolerated stage
//! failure (`collect_managed_surface`'s `stage_failures`) that made this run's surface incomplete
//! rather than the file genuinely dropped. A missed report leaves a stale file a human eventually
//! notices, exactly the status quo this module improves on; a wrong deletion destroys work with
//! no undo. The asymmetry is the same one `report_disk_scan_candidates`
//! (`src/cli/pipeline/generate/orphans.rs`) already documents for the build-time disk-scan
//! route -- this module is `alef verify`'s counterpart, not a duplicate: it runs over the whole
//! tree `collect_alef_hashes` walks, using the full multi-stage managed surface as ground
//! truth, rather than one backend's own output root. ~keep

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Workspace-root paths a scaffold stage emits **only when the path is absent** (see
/// `src/scaffold/mod.rs`'s `rust_toolchain_file` and `src/scaffold/languages/wasm.rs`'s
/// `wasm_cargo_config_file`, both gated on `!Path::new(<path>).exists()`). Once created, such a
/// file falls outside every later run's
/// in-memory `surface` by design -- it is meant to become user-owned after the first scaffold,
/// so nothing regenerates or reports it missing if deleted either (see `missing_managed_paths`,
/// which reads the same `surface`). Diffing that surface against disk would misreport this exact
/// file as an orphan on every single `alef verify` run for every consumer who has one, which is
/// not a rare edge case -- it is the common case, not a rare one, so it is excluded by name
/// rather than left to read as a false alarm. Narrow and explicit by construction, the same way
/// `UNMARKABLE_ALEF_MANIFESTS` in `src/cli/pipeline/generate/orphans.rs` is: extend only when a
/// new scaffold call site adds the identical `!exists()` gate, verified at the call site first. ~keep
const CREATE_ONCE_SEED_PATHS: &[&str] = &["rust-toolchain.toml", ".cargo/config.toml"];

/// True when `path` (absolute, under `base_dir`) is a known create-once seed -- see
/// [`CREATE_ONCE_SEED_PATHS`].
fn is_create_once_seed(base_dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(base_dir) else {
        return false;
    };
    let Some(relative) = relative.to_str() else {
        return false;
    };
    CREATE_ONCE_SEED_PATHS.contains(&relative)
}

/// Absolute, display-formatted paths of every alef-marked file under `base_dir` that is not in
/// `managed_paths` -- i.e. a file the current run's backends would not produce today.
///
/// `managed_paths` must be the union, across every crate in a (possibly multi-crate) workspace,
/// of every path that crate's managed surface would touch this run (see
/// [`super::helpers::MissingAndFrozenFiles::managed_paths`]). Unioning across crates before
/// calling this, rather than diffing crate by crate, is required for correctness: a file crate B
/// legitimately owns would look orphaned if only crate A's managed paths were checked against
/// it.
///
/// Only files [`super::helpers::collect_alef_hashes`] itself would return are candidates, so a
/// user-owned file that happens to sit in a generated directory -- no `alef:hash:`/self-marker
/// header, ever -- can never appear here; ownership is decided purely by the marker, the same
/// predicate every other `alef verify` finding uses. A known [`CREATE_ONCE_SEED_PATHS`] entry is
/// excluded even when absent from `managed_paths`, for the reason documented on that constant.
pub(crate) fn find_orphaned_generated_files(
    base_dir: &Path,
    managed_paths: &HashSet<PathBuf>,
    declared_user_owned: &crate::core::config::UserOwnedPaths,
) -> Vec<String> {
    let mut orphans: Vec<String> = super::helpers::collect_alef_hashes(base_dir)
        .into_iter()
        .filter(|(path, _hash, _content)| {
            // ~keep A path the repository declared under `[workspace.ownership] user_owned` is
            // by definition absent from the managed surface -- alef stops producing it, which
            // is the whole point of the declaration -- so without this it lands here as an
            // orphan and `has_orphan_files` gates the exit code. That made the declaration
            // useless for the case it exists to fix: the consumer moves a permanently-failing
            // path out of "stale"/"frozen" and it reappears under "orphaned", still failing,
            // still with no reachable remedy. `OwnershipConfig`'s module doc promises such a
            // path is "counted as a declared skip rather than a failure"; this is where that
            // promise has to hold.
            !declared_user_owned.matches(base_dir, path)
                && !managed_paths.contains(path)
                && !is_create_once_seed(base_dir, path)
        })
        .map(|(path, _hash, _content)| path.display().to_string())
        .collect();
    orphans.sort();
    orphans.dedup();
    orphans
}

/// Every path a configured post-build step writes unguarded, across every language in
/// `languages` -- see [`crate::core::backend::PostBuildStep::owned_paths`].
///
/// `alef verify` never runs post-build steps: `complete_generated_artifacts` is invoked only
/// from `Commands::Generate`/`Commands::All` (`bin_cli::core_commands`), so a path one of these
/// steps owns can never appear in `collect_managed_surface`'s in-memory surface the way a
/// `GeneratedFile` would. Left out of the set passed to [`find_orphaned_generated_files`], such a
/// path would misreport as an orphan on every single `alef verify` run the moment a post-build
/// step plants an alef-marked file there -- the same false positive `Commands::Generate`'s own
/// orphan sweep (`bin_cli::core_commands`) already avoids by folding this exact union in before
/// its disk-scan diff (see `owned_paths`'s doc for the alef #B incident that guards against). No
/// shipped backend's `owned_paths` returns a marker-carrying path today, so the gap is latent
/// rather than live here too, but closing it keeps `alef verify` correct the moment one does. ~keep
pub(crate) fn post_build_owned_paths(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &Path,
) -> HashSet<PathBuf> {
    let mut owned = HashSet::new();
    for &language in languages {
        let Some(backend) = crate::cli::registry::try_get_backend(language) else {
            continue;
        };
        let Some(build_config) = backend.build_config_with_config(config) else {
            continue;
        };
        owned.extend(
            build_config
                .post_build
                .iter()
                .flat_map(|step| step.owned_paths(base_dir)),
        );
    }
    owned
}

#[cfg(test)]
mod tests;
