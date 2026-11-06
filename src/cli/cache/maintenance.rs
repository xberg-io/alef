//! Cache and ownership-record maintenance: the destructive operations that keep `.alef/` and
//! `.alef-ownership.toml` from growing forever.
//!
//! Split out of `cache.rs` for the file-modularization cap. Nothing here computes a cache key or
//! decides a skip; it only removes entries that can no longer skip or prove anything.

use std::fs;
use std::path::Path;

use super::{
    CACHE_DIR, OwnedPathsRecord, PER_FILE_CACHE_NAME, ownership_manifest_path, read_owned_paths_record,
    render_ownership_manifest,
};

/// Clear every cache entry that can skip generation work, without deleting the `.alef/` records
/// that are not caches (`last_synced_version`, snippet sessions).
///
/// `--clean` on `generate`/`all` means "recompute every stage from source". Bypassing the read on
/// this run is not enough: the entries it leaves behind are read by a *different* subcommand
/// afterwards (`alef stubs` right after `alef generate --clean` was the reported case), and a
/// cache written by an older build then answers "up to date" for the new one. Clearing the shared
/// cache here keeps the flag's promise across commands.
pub fn clear_generation_cache() -> anyhow::Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    if !cache_dir.exists() {
        return Ok(());
    }
    let hashes = cache_dir.join("hashes");
    if hashes.exists() {
        fs::remove_dir_all(&hashes)?;
    }
    let memo = cache_dir.join(PER_FILE_CACHE_NAME);
    if memo.exists() {
        fs::remove_file(&memo)?;
    }
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            // Per-crate directories hold `ir.json`, `ir.hash`, and the language manifests. ~keep
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

/// Drop ownership entries whose file is gone, for every entry under `roots`.
///
/// `owned_paths` is a historical claim, but a claim about a path alef no longer emits and that is
/// no longer on disk is stale bookkeeping: nothing reads it except `alef verify` (which could not
/// tell it apart from a live claim) and the write guard (which never needs it for an absent file --
/// creating a missing path needs no historical proof). Removing those entries is what keeps
/// `.alef-ownership.toml` from accumulating dead paths when a fixture or language is dropped and
/// its output sweeps away. A file that is still present is never touched, and an entry outside
/// every `roots` is left alone because it was not this run's output to reason about.
///
/// `roots` may be absolute or `base_dir`-relative. A missing or unreadable record is a no-op, not
/// an error: this runs after generation, and a bookkeeping repair must not turn a successful run
/// into a failure. A no-op, byte-for-byte, when nothing is pruned.
pub fn prune_absent_owned_paths(base_dir: &Path, roots: &[&Path]) -> anyhow::Result<usize> {
    if roots.is_empty() {
        return Ok(0);
    }
    let relative_roots: Vec<&Path> = roots
        .iter()
        .map(|root| root.strip_prefix(base_dir).unwrap_or(root))
        .collect();
    let OwnedPathsRecord::Present(paths) = read_owned_paths_record(base_dir) else {
        return Ok(0);
    };
    let (kept, pruned): (Vec<String>, Vec<String>) = paths.into_iter().partition(|entry| {
        let path = Path::new(entry);
        let under_root = relative_roots.iter().any(|root| path.starts_with(root));
        !under_root || base_dir.join(path).exists()
    });
    if pruned.is_empty() {
        return Ok(0);
    }
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    fs::write(ownership_manifest_path(base_dir), render_ownership_manifest(&kept))?;
    Ok(pruned.len())
}
