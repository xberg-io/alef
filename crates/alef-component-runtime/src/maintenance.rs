//! Cache inspection and cleanup: listing, removal and pruning of installed objects, plus
//! removal of crash leftovers (`.install-*` staging dirs, orphaned temp files, stale
//! `.lock`, `.used` and `.verified` files).

use crate::ArtifactCache;
use crate::ComponentError;
use fs2::FileExt as _;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Staging directories and temp files younger than this may belong to a live install in
/// another process and are left alone.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(60 * 60);
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// One installed object in the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CachedObject {
    /// The archive digest naming the object directory.
    pub sha256: String,
    pub path: PathBuf,
    /// Total size of the files under `path`.
    pub size_bytes: u64,
    /// When the object was last loaded through a cache (falls back to install time).
    pub last_used: Option<SystemTime>,
    /// The component id in this manager's lock that references the object, when any.
    pub component: Option<String>,
}

/// Which objects [`ComponentManager::prune`](crate::ComponentManager::prune) keeps. An object
/// is kept when it satisfies any enabled criterion; with none enabled everything is removed.
///
/// Several host versions can share one cache, so the usual policy combines both criteria:
/// keep what this lock needs plus whatever any host used recently.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Retain {
    referenced_by_lock: bool,
    used_within: Option<Duration>,
}

impl Retain {
    /// Keep nothing (until criteria are added).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep objects referenced by the manager's lock (every target in it).
    #[must_use]
    pub fn referenced_by_lock(mut self) -> Self {
        self.referenced_by_lock = true;
        self
    }

    /// Keep objects loaded within the last `days` days.
    #[must_use]
    pub fn used_within_days(self, days: u64) -> Self {
        self.used_within(Duration::from_secs(days.saturating_mul(SECONDS_PER_DAY)))
    }

    /// Keep objects loaded within the last `window`.
    #[must_use]
    pub fn used_within(mut self, window: Duration) -> Self {
        self.used_within = Some(window);
        self
    }

    fn keeps(&self, referenced: bool, last_used: Option<SystemTime>, now: SystemTime) -> bool {
        if self.referenced_by_lock && referenced {
            return true;
        }
        match (self.used_within, last_used) {
            (Some(window), Some(used)) => !now.duration_since(used).is_ok_and(|age| age > window),
            _ => false,
        }
    }
}

/// A path a cleanup could not remove, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedRemoval {
    pub path: PathBuf,
    pub reason: String,
}

/// What a removal or prune did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemovalReport {
    pub removed: Vec<PathBuf>,
    /// Objects left in place: loaded in this process, or (Windows) held open by a loaded
    /// library so the OS refused the delete.
    pub skipped: Vec<SkippedRemoval>,
    pub freed_bytes: u64,
}

impl ArtifactCache {
    pub(crate) fn objects_dir(&self) -> PathBuf {
        self.root.join("sha256")
    }

    fn used_path(&self, sha256: &str) -> PathBuf {
        self.objects_dir().join(format!("{sha256}.used"))
    }

    /// Record that `sha256` was just loaded. Best effort.
    pub(crate) fn touch_used(&self, sha256: &str) {
        let Ok(file) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.used_path(sha256))
        else {
            return;
        };
        let _ = file.set_modified(SystemTime::now());
    }

    /// Every installed object, sorted by digest. A missing cache directory lists as empty.
    pub fn list_objects(&self) -> Result<Vec<CachedObject>, ComponentError> {
        let objects = self.objects_dir();
        let reader = match fs::read_dir(&objects) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut listed = Vec::new();
        for item in reader {
            let item = item?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !is_digest_name(&name) || !item.file_type()?.is_dir() {
                continue;
            }
            let path = item.path();
            let installed = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
            let used = fs::metadata(self.used_path(&name))
                .and_then(|meta| meta.modified())
                .ok();
            listed.push(CachedObject {
                size_bytes: dir_size(&path),
                last_used: used.or(installed),
                sha256: name,
                path,
                component: None,
            });
        }
        listed.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        Ok(listed)
    }

    /// Remove one object and its sidecar files. `Ok` with an empty report when absent.
    pub fn remove_object(&self, sha256: &str) -> Result<RemovalReport, ComponentError> {
        crate::cache::decode_digest(sha256)?;
        let mut report = RemovalReport::default();
        let path = self.objects_dir().join(sha256);
        if path.exists() {
            let size = dir_size(&path);
            match fs::remove_dir_all(&path) {
                Ok(()) => {
                    report.removed.push(path);
                    report.freed_bytes = size;
                }
                Err(error) if is_sharing_violation(&error) => {
                    report.skipped.push(SkippedRemoval {
                        path,
                        reason: format!("in use by a loaded library: {error}"),
                    });
                    return Ok(report);
                }
                Err(error) => return Err(error.into()),
            }
        }
        for suffix in ["used", "verified"] {
            let _ = fs::remove_file(self.objects_dir().join(format!("{sha256}.{suffix}")));
        }
        self.remove_lock_if_idle(sha256);
        Ok(report)
    }

    /// Remove every object `retain` does not keep, except those in `protected` (digests
    /// that are loaded in this process), then sweep crash leftovers.
    pub(crate) fn prune_objects(
        &self,
        retain: &Retain,
        locked: &HashSet<String>,
        protected: &HashSet<String>,
        now: SystemTime,
    ) -> Result<RemovalReport, ComponentError> {
        let mut report = RemovalReport::default();
        for object in self.list_objects()? {
            if retain.keeps(locked.contains(&object.sha256), object.last_used, now) {
                continue;
            }
            if protected.contains(&object.sha256) {
                report.skipped.push(SkippedRemoval {
                    path: object.path,
                    reason: "loaded in this process".to_owned(),
                });
                continue;
            }
            let removed = self.remove_object(&object.sha256)?;
            report.removed.extend(removed.removed);
            report.skipped.extend(removed.skipped);
            report.freed_bytes += removed.freed_bytes;
        }
        let swept = self.clean_leftovers(STALE_AFTER, now)?;
        report.removed.extend(swept.removed);
        report.freed_bytes += swept.freed_bytes;
        Ok(report)
    }

    /// Delete `.install-*` staging dirs and `.tmp*` files older than `min_age`, and
    /// `.lock` / `.used` / `.verified` sidecars whose object is gone (a `.lock` only when no
    /// process holds it).
    pub(crate) fn clean_leftovers(&self, min_age: Duration, now: SystemTime) -> Result<RemovalReport, ComponentError> {
        let mut report = RemovalReport::default();
        let objects = self.objects_dir();
        let reader = match fs::read_dir(&objects) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(error) => return Err(error.into()),
        };
        for item in reader {
            let item = item?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = item.path();
            let is_dir = item.file_type()?.is_dir();
            let old_enough = || {
                fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= min_age)
            };
            let staging = (name.starts_with(".install-") && is_dir) || (name.starts_with(".tmp") && !is_dir);
            let stale = if staging {
                old_enough()
            } else if let Some(digest) = sidecar_digest(&name) {
                !is_dir && !objects.join(digest).is_dir() && self.sidecar_is_unclaimed(&path, &name)
            } else {
                false
            };
            if !stale {
                continue;
            }
            let size = if is_dir {
                dir_size(&path)
            } else {
                item.metadata().map_or(0, |meta| meta.len())
            };
            let result = if is_dir {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    report.removed.push(path);
                    report.freed_bytes += size;
                }
                Err(error) if is_sharing_violation(&error) => report.skipped.push(SkippedRemoval {
                    path,
                    reason: error.to_string(),
                }),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(report)
    }

    /// A `.lock` file is only stale when no process holds it; other sidecars always are.
    fn sidecar_is_unclaimed(&self, path: &Path, name: &str) -> bool {
        if !name.ends_with(".lock") {
            return true;
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_ok_and(|file| file.try_lock_exclusive().is_ok())
    }

    fn remove_lock_if_idle(&self, sha256: &str) {
        let path = self.objects_dir().join(format!("{sha256}.lock"));
        if self.sidecar_is_unclaimed(&path, &format!("{sha256}.lock")) {
            let _ = fs::remove_file(path);
        }
    }
}

fn is_digest_name(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The digest a `<digest>.lock|used|verified` sidecar name refers to.
fn sidecar_digest(name: &str) -> Option<&str> {
    let (digest, suffix) = name.split_once('.')?;
    (is_digest_name(digest) && matches!(suffix, "lock" | "used" | "verified")).then_some(digest)
}

fn dir_size(path: &Path) -> u64 {
    let Ok(reader) = fs::read_dir(path) else {
        return 0;
    };
    reader
        .flatten()
        .map(|item| match item.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&item.path()),
            Ok(_) => item.metadata().map_or(0, |meta| meta.len()),
            Err(_) => 0,
        })
        .sum()
}

/// Windows refuses to delete a loaded DLL (access denied or sharing/lock violation).
fn is_sharing_violation(error: &std::io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    cfg!(windows)
        && (error.kind() == std::io::ErrorKind::PermissionDenied
            || matches!(
                error.raw_os_error(),
                Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrustPolicy;

    fn cache(dir: &Path) -> ArtifactCache {
        ArtifactCache::new(dir, TrustPolicy::DigestOnly)
    }

    fn digest(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    fn make_object(cache: &ArtifactCache, byte: u8) -> PathBuf {
        let path = cache.objects_dir().join(digest(byte));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("lib.bin"), b"12345").unwrap();
        path
    }

    fn age(path: &Path, days: u64) {
        let past = SystemTime::now() - Duration::from_secs(days * SECONDS_PER_DAY);
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(past)
            .unwrap();
    }

    #[test]
    fn lists_objects_with_size_and_ignores_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        make_object(&cache, 1);
        fs::write(cache.objects_dir().join(format!("{}.lock", digest(1))), b"").unwrap();
        fs::create_dir_all(cache.objects_dir().join(".install-abc")).unwrap();

        let listed = cache.list_objects().unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].sha256, digest(1));
        assert_eq!(listed[0].size_bytes, 5);
        assert!(listed[0].last_used.is_some());
    }

    #[test]
    fn listing_a_missing_cache_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cache(&dir.path().join("nope")).list_objects().unwrap().is_empty());
    }

    #[test]
    fn remove_object_deletes_directory_and_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let path = make_object(&cache, 1);
        cache.touch_used(&digest(1));
        fs::write(cache.objects_dir().join(format!("{}.lock", digest(1))), b"").unwrap();

        let report = cache.remove_object(&digest(1)).unwrap();

        assert_eq!(report.removed, vec![path.clone()]);
        assert_eq!(report.freed_bytes, 5);
        assert!(!path.exists());
        assert!(!cache.used_path(&digest(1)).exists());
        assert!(!cache.objects_dir().join(format!("{}.lock", digest(1))).exists());
        assert!(cache.remove_object(&digest(1)).unwrap().removed.is_empty());
    }

    #[test]
    fn prune_keeps_locked_or_recently_used_and_removes_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let locked = make_object(&cache, 1);
        let recent = make_object(&cache, 2);
        let old = make_object(&cache, 3);
        for byte in [1, 2, 3] {
            cache.touch_used(&digest(byte));
        }
        age(&cache.used_path(&digest(1)), 90);
        age(&cache.used_path(&digest(3)), 90);
        let referenced = HashSet::from([digest(1)]);
        let retain = Retain::new().referenced_by_lock().used_within_days(30);

        let report = cache
            .prune_objects(&retain, &referenced, &HashSet::new(), SystemTime::now())
            .unwrap();

        assert!(locked.exists() && recent.exists());
        assert!(!old.exists());
        assert_eq!(report.removed, vec![old]);
    }

    #[test]
    fn prune_with_no_criteria_removes_everything_but_skips_protected() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let loaded = make_object(&cache, 1);
        let other = make_object(&cache, 2);

        let report = cache
            .prune_objects(
                &Retain::new(),
                &HashSet::new(),
                &HashSet::from([digest(1)]),
                SystemTime::now(),
            )
            .unwrap();

        assert!(loaded.exists());
        assert!(!other.exists());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].path, loaded);
    }

    #[test]
    fn clean_leftovers_removes_stale_staging_and_orphaned_sidecars_only() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let objects = cache.objects_dir();
        let kept = make_object(&cache, 1);
        fs::write(objects.join(format!("{}.lock", digest(1))), b"").unwrap();
        let staging = objects.join(".install-crashed");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("partial"), b"xx").unwrap();
        let orphan_lock = objects.join(format!("{}.lock", digest(9)));
        fs::write(&orphan_lock, b"").unwrap();
        let orphan_stamp = objects.join(format!("{}.verified", digest(9)));
        fs::write(&orphan_stamp, b"{}").unwrap();

        let fresh = cache
            .clean_leftovers(Duration::from_secs(3600), SystemTime::now())
            .unwrap();
        assert!(staging.exists(), "a young staging dir may belong to a live install");
        assert!(!orphan_lock.exists() && !orphan_stamp.exists());
        assert_eq!(fresh.removed.len(), 2);

        let later = cache.clean_leftovers(Duration::ZERO, SystemTime::now()).unwrap();
        assert!(!staging.exists());
        assert_eq!(later.removed, vec![staging]);
        assert_eq!(later.freed_bytes, 2);
        assert!(kept.exists());
        assert!(objects.join(format!("{}.lock", digest(1))).exists());
    }

    #[test]
    fn held_lock_file_is_not_treated_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        fs::create_dir_all(cache.objects_dir()).unwrap();
        let lock_path = cache.objects_dir().join(format!("{}.lock", digest(9)));
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        held.lock_exclusive().unwrap();

        cache.clean_leftovers(Duration::ZERO, SystemTime::now()).unwrap();

        assert!(lock_path.exists());
    }
}
