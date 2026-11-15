//! Binding the code the runtime loads or executes to the file it verified.
//!
//! The cache verifies a library through one open descriptor ([`VerifiedFile`]). Whatever is loaded
//! or executed afterwards must be that file, not whatever its path names by then:
//!
//! * On Linux a library is opened with `dlopen("/proc/self/fd/N")` on the verified descriptor, and
//!   an ELF sidecar executable is started from the same path, so replacing the path after
//!   verification cannot change what runs.
//! * Elsewhere, when `/proc` is not mounted, and for a script sidecar (an interpreter cannot read
//!   a close-on-exec descriptor), the path is re-checked immediately before `dlopen` or spawn and
//!   refused with [`ComponentError::ArtifactChanged`] when its device, inode, size or modification
//!   time differ from the verified descriptor's (Windows compares size and modification time).
//!   A replacement between that check and the load is not caught; making it requires write access
//!   to the cache's object directory, which the cache creates private to the current user.
//!
//! ADR-0026 records the decision.

use crate::ComponentError;
use std::fs::{File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// The parts of a file's metadata that change when the file at a path is replaced or rewritten.
/// The change time is deliberately absent: making a sidecar executable changes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    dev: u64,
    ino: u64,
    size: u64,
    modified: Option<SystemTime>,
}

impl FileIdentity {
    pub(crate) fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        let (dev, ino) = {
            use std::os::unix::fs::MetadataExt as _;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let (dev, ino) = (0, 0);
        Self {
            dev,
            ino,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

/// A library or executable whose bytes were verified through `file`, kept open so the same file
/// can be loaded later.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedFile {
    path: PathBuf,
    file: Arc<File>,
    identity: FileIdentity,
}

impl VerifiedFile {
    /// `metadata` must be the metadata of `file`, read before its contents were verified.
    pub(crate) fn new(path: PathBuf, file: File, metadata: &Metadata) -> Self {
        Self {
            path,
            file: Arc::new(file),
            identity: FileIdentity::of(metadata),
        }
    }

    /// The same descriptor under the path a rename moved it to; a rename keeps the inode.
    pub(crate) fn relocated(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The verified file's current metadata, read from the descriptor rather than the path.
    pub(crate) fn metadata(&self) -> std::io::Result<Metadata> {
        self.file.metadata()
    }

    /// Fail unless the path still names the verified file.
    pub(crate) fn ensure_unchanged(&self) -> Result<(), ComponentError> {
        match std::fs::metadata(&self.path) {
            Ok(metadata) if FileIdentity::of(&metadata) == self.identity => Ok(()),
            _ => Err(ComponentError::ArtifactChanged(self.path.clone())),
        }
    }

    /// The path to pass to `dlopen`. Call it immediately before loading: on platforms without a
    /// descriptor path it performs the last identity check.
    pub(crate) fn library_load_path(&self) -> Result<PathBuf, ComponentError> {
        if let Some(path) = self.descriptor_path() {
            return Ok(path);
        }
        self.ensure_unchanged()?;
        Ok(self.path.clone())
    }

    /// The program path to execute. Call it immediately before spawning, for the same reason as
    /// [`Self::library_load_path`].
    pub(crate) fn executable_launch_path(&self) -> Result<PathBuf, ComponentError> {
        if self.is_elf()
            && let Some(path) = self.descriptor_path()
        {
            return Ok(path);
        }
        self.ensure_unchanged()?;
        Ok(self.path.clone())
    }

    #[cfg(target_os = "linux")]
    fn descriptor_path(&self) -> Option<PathBuf> {
        use std::os::fd::AsRawFd as _;
        Path::new("/proc/self/fd")
            .is_dir()
            .then(|| PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd())))
    }

    #[cfg(not(target_os = "linux"))]
    fn descriptor_path(&self) -> Option<PathBuf> {
        None
    }

    /// Whether the verified file is an ELF image, which the kernel can execute from a descriptor
    /// path. A `#!` script cannot be: its interpreter reopens the path after the close-on-exec
    /// descriptor is gone.
    #[cfg(target_os = "linux")]
    fn is_elf(&self) -> bool {
        use std::os::unix::fs::FileExt as _;
        let mut magic = [0_u8; 4];
        self.file.read_exact_at(&mut magic, 0).is_ok() && magic == *b"\x7fELF"
    }

    #[cfg(not(target_os = "linux"))]
    fn is_elf(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified(dir: &Path, bytes: &[u8]) -> VerifiedFile {
        let path = dir.join("lib.bin");
        std::fs::write(&path, bytes).unwrap();
        let file = File::open(&path).unwrap();
        let metadata = file.metadata().unwrap();
        VerifiedFile::new(path, file, &metadata)
    }

    #[test]
    fn an_untouched_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let verified = verified(dir.path(), b"native");
        verified.ensure_unchanged().unwrap();
        verified.library_load_path().unwrap();
    }

    #[test]
    fn a_replaced_path_is_refused_before_loading() {
        let dir = tempfile::tempdir().unwrap();
        let verified = verified(dir.path(), b"native");
        let replacement = dir.path().join("evil.bin");
        std::fs::write(&replacement, b"evil!!!!").unwrap();
        std::fs::rename(&replacement, verified.path()).unwrap();

        assert!(matches!(
            verified.ensure_unchanged(),
            Err(ComponentError::ArtifactChanged(path)) if path == verified.path()
        ));
        // A script is always launched by path, so the swap is refused there on every platform.
        assert!(matches!(
            verified.executable_launch_path(),
            Err(ComponentError::ArtifactChanged(_))
        ));
    }

    /// On Linux the library is loaded from the verified descriptor itself, so a path swap after
    /// verification is harmless: the descriptor path still reads the verified bytes.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_loads_through_the_verified_descriptor_even_after_a_swap() {
        let dir = tempfile::tempdir().unwrap();
        let verified = verified(dir.path(), b"native");
        let replacement = dir.path().join("evil.bin");
        std::fs::write(&replacement, b"evil!!!!").unwrap();
        std::fs::rename(&replacement, verified.path()).unwrap();

        let load_path = verified.library_load_path().unwrap();
        assert!(load_path.starts_with("/proc/self/fd"), "{}", load_path.display());
        assert_eq!(std::fs::read(load_path).unwrap(), b"native");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_launches_elf_executables_through_the_descriptor_and_scripts_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let elf = verified(dir.path(), b"\x7fELF-rest-of-image");
        assert!(elf.executable_launch_path().unwrap().starts_with("/proc/self/fd"));

        let script_dir = tempfile::tempdir().unwrap();
        let script = verified(script_dir.path(), b"#!/bin/sh\n");
        assert_eq!(script.executable_launch_path().unwrap(), script.path());
    }
}
