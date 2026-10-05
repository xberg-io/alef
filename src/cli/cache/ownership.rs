//! Whether alef has ever recorded owning a given path -- the query side of the ownership
//! manifest, split out of `cache.rs` purely to keep that already-oversized file from growing
//! further (see the `file-modularization` rule). The write side
//! ([`super::record_scaffold_owned_path`], [`super::record_scaffold_owned_paths`]) and the
//! records themselves stay in the parent module.

use std::path::Path;

use super::{
    note_untracked_required_records, read_committed_owned_paths, read_legacy_owned_paths, record_scaffold_owned_paths,
    scaffold_owned_path_key,
};

#[cfg(test)]
mod tests;

/// True only when the committed `.alef-ownership.toml` records `path` for `base_dir`.
///
/// Fresh-render verification uses this narrower query because content verification must be
/// reproducible in a fresh clone. The legacy gitignored record remains accepted by the write
/// guard through [`is_scaffold_owned_path`], but it is local machine state rather than durable
/// evidence that may support a release gate. ~keep
pub(crate) fn is_committed_scaffold_owned_path(base_dir: &Path, path: &Path) -> bool {
    note_untracked_required_records(base_dir);
    let key = scaffold_owned_path_key(base_dir, path);
    read_committed_owned_paths(base_dir).contains(&key)
}

/// True when `path` was previously recorded by [`super::record_scaffold_owned_path`]
/// for this `base_dir`.
///
/// Reads the committed [`super::OWNERSHIP_MANIFEST`] unioned with the legacy
/// gitignored record (see [`super::LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST`]). Once the
/// committed manifest is in the repository this answers identically on a fresh
/// clone and on a warm machine, which is the whole point of moving it; the
/// legacy half is the only remaining source of machine-local divergence and it
/// can only ever say `true` where the old code already did. When neither knows
/// the path the answer is `false` and the write-time guard refuses rather than
/// risk clobbering foreign content.
///
/// Also eagerly promotes any legacy-only entries into the committed manifest --
/// see [`migrate_legacy_owned_paths`]. That closes the window
/// [`super::LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST`]'s doc concedes is still open: a path whose
/// ownership predates #80 and has never been rewritten since lives only under the
/// gitignored cache until some future write happens to touch that exact path, which for
/// a static file may never come. Every ownership-gated write and `alef verify`'s
/// frozen-file scan already call this function unconditionally for every unmarkable
/// managed path, so a single ordinary run promotes the whole legacy manifest before the
/// cache can be cleared out from under it -- after which clearing `.alef` is harmless. ~keep
pub fn is_scaffold_owned_path(base_dir: &Path, path: &Path) -> bool {
    note_untracked_required_records(base_dir);
    let key = scaffold_owned_path_key(base_dir, path);
    let committed = read_committed_owned_paths(base_dir);
    let legacy = read_legacy_owned_paths(base_dir);
    migrate_legacy_owned_paths(base_dir, &legacy, &committed);
    committed.iter().chain(legacy.iter()).any(|existing| *existing == key)
}

/// Promote every entry in `legacy` not already present in `committed` into the committed
/// [`super::OWNERSHIP_MANIFEST`], so a query answers from durable, committable state from then on
/// rather than depending on the gitignored legacy cache surviving.
///
/// Best-effort and silent on failure: this runs inside a boolean *query*
/// ([`is_scaffold_owned_path`]), which has no channel to report a write failure through
/// and must never turn a read into a hard error. An unreadable committed manifest is left
/// untouched by [`record_scaffold_owned_paths`] itself (it refuses rather than risk
/// dropping paths already recorded there), so the worst outcome of a failed migration is
/// exactly today's behaviour: the entry keeps answering `true` from the legacy cache
/// alone, for as long as that cache survives.
///
/// A no-op, and therefore cheap, once every legacy path has already been migrated once
/// in this repository -- the common case for every query after the first. ~keep
fn migrate_legacy_owned_paths(base_dir: &Path, legacy: &[String], committed: &[String]) {
    if legacy.is_empty() {
        return;
    }
    let committed_set: std::collections::HashSet<&str> = committed.iter().map(String::as_str).collect();
    let to_migrate: Vec<&Path> = legacy
        .iter()
        .filter(|path| !committed_set.contains(path.as_str()))
        .map(|path| Path::new(path.as_str()))
        .collect();
    if to_migrate.is_empty() {
        return;
    }
    if let Err(error) = record_scaffold_owned_paths(base_dir, &to_migrate) {
        tracing::warn!(
            reason = %error,
            "failed to migrate legacy ownership record entries into the committed manifest; \
             they remain valid from the legacy cache alone until it is cleared"
        );
    }
}
