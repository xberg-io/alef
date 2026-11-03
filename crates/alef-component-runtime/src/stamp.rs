//! Verified stamp: a cheap on-disk record that an installed library was fully checked.
//!
//! After a full SHA-256 check of an installed library, [`record`] writes a small JSON stamp
//! next to the object. A later process (see
//! [`ArtifactCache::with_verified_stamp`](crate::ArtifactCache::with_verified_stamp)) trusts a
//! stamp whose recorded file identity still matches the library and skips re-hashing it.
//!
//! # Threat model
//!
//! The stamp binds the archive digest, the library's size, modification time (nanosecond
//! resolution) and, on Unix, device, inode and change time (ctime). It is invalidated when
//! the library is replaced (new inode), truncated or extended (size), or rewritten through
//! normal means (mtime and ctime). The manifest digest, canonical form, identity and
//! signature are still checked on every load; only the library read and hash is skipped.
//!
//! It does not defend against an attacker who can already write the cache directory and who
//! edits the library in place without changing its size while restoring the mtime. On Unix
//! that attacker must also forge the ctime, which requires changing the system clock or
//! writing the raw device; on Windows only size and mtime are recorded, so a same-size
//! in-place edit with a restored mtime is not caught. Such an attacker can equally rewrite
//! the stamp itself or the library the process is about to load, so the stamp trades full
//! re-verification for startup latency, not for protection against local cache writers.
//! Leave it disabled (the default) when the cache directory is shared with less-trusted
//! users.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::UNIX_EPOCH;

const STAMP_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VerifiedStamp {
    version: u32,
    archive_sha256: String,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    dev: Option<u64>,
    ino: Option<u64>,
    ctime_secs: Option<i64>,
    ctime_nanos: Option<i64>,
}

/// Snapshot the identity of `library` as it is on disk right now.
pub(crate) fn capture(archive_sha256: &str, library: &Path) -> std::io::Result<VerifiedStamp> {
    let metadata = fs::metadata(library)?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    #[cfg(unix)]
    let (dev, ino, ctime_secs, ctime_nanos) = {
        use std::os::unix::fs::MetadataExt as _;
        (
            Some(metadata.dev()),
            Some(metadata.ino()),
            Some(metadata.ctime()),
            Some(metadata.ctime_nsec()),
        )
    };
    #[cfg(not(unix))]
    let (dev, ino, ctime_secs, ctime_nanos) = (None, None, None, None);
    Ok(VerifiedStamp {
        version: STAMP_VERSION,
        archive_sha256: archive_sha256.to_owned(),
        size: metadata.len(),
        mtime_secs: modified.as_secs(),
        mtime_nanos: modified.subsec_nanos(),
        dev,
        ino,
        ctime_secs,
        ctime_nanos,
    })
}

/// Whether the stamp at `stamp_path` matches `library`'s current identity.
pub(crate) fn is_current(stamp_path: &Path, archive_sha256: &str, library: &Path) -> bool {
    let Ok(bytes) = fs::read(stamp_path) else {
        return false;
    };
    let Ok(stored) = serde_json::from_slice::<VerifiedStamp>(&bytes) else {
        return false;
    };
    capture(archive_sha256, library).is_ok_and(|current| current == stored)
}

/// Atomically write `stamp` to `stamp_path`. Best effort: failures leave no stamp.
pub(crate) fn write(stamp_path: &Path, stamp: &VerifiedStamp) {
    let Some(parent) = stamp_path.parent() else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(stamp) else {
        return;
    };
    let Ok(mut file) = tempfile::Builder::new().prefix(".tmp-stamp-").tempfile_in(parent) else {
        return;
    };
    if file.write_all(&bytes).is_ok() {
        let _ = file.persist(stamp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("lib.bin");
        fs::write(&path, b"native").unwrap();
        path
    }

    #[test]
    fn matches_unchanged_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let stamp_path = dir.path().join("lib.verified");
        write(&stamp_path, &capture("aa", &lib).unwrap());
        assert!(is_current(&stamp_path, "aa", &lib));
    }

    #[test]
    fn rejects_other_archive_digest_missing_stamp_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let stamp_path = dir.path().join("lib.verified");
        assert!(!is_current(&stamp_path, "aa", &lib));
        write(&stamp_path, &capture("aa", &lib).unwrap());
        assert!(!is_current(&stamp_path, "bb", &lib));
        fs::write(&stamp_path, b"not json").unwrap();
        assert!(!is_current(&stamp_path, "aa", &lib));
    }

    #[test]
    fn rejects_size_change() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let stamp_path = dir.path().join("lib.verified");
        write(&stamp_path, &capture("aa", &lib).unwrap());
        fs::write(&lib, b"native-longer").unwrap();
        assert!(!is_current(&stamp_path, "aa", &lib));
    }

    #[test]
    fn rejects_same_size_edit_with_new_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let stamp_path = dir.path().join("lib.verified");
        write(&stamp_path, &capture("aa", &lib).unwrap());
        fs::write(&lib, b"nativf").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&lib)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(!is_current(&stamp_path, "aa", &lib));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_replaced_file_even_with_identical_size_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let stamp_path = dir.path().join("lib.verified");
        let original = fs::metadata(&lib).unwrap().modified().unwrap();
        write(&stamp_path, &capture("aa", &lib).unwrap());
        // Keep the old inode alive so the replacement cannot reuse it.
        let _keep = fs::File::open(&lib).unwrap();
        let replacement = dir.path().join("replacement.bin");
        fs::write(&replacement, b"evil!!").unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_modified(original)
            .unwrap();
        fs::rename(&replacement, &lib).unwrap();
        assert!(!is_current(&stamp_path, "aa", &lib));
    }
}
