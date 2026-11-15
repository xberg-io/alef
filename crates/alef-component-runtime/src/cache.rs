use crate::download::{DownloadOptions, TrackedReader, is_transient_request_error};
use crate::identity::{FileIdentity, VerifiedFile};
use crate::keys::decode_public_key;
use crate::manifest::{COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA};
use crate::stamp;
use crate::{ComponentError, ComponentLockEntry, ComponentManifest, ComponentSignature};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier as _};
use flate2::read::GzDecoder;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek as _, SeekFrom, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Rewrites a lock entry's download URL at runtime; see [`ArtifactCache::with_url_override`].
pub(crate) type UrlOverride = Arc<dyn Fn(&ComponentLockEntry) -> String + Send + Sync>;

#[derive(Clone, Debug)]
pub enum TrustPolicy {
    DigestOnly,
    EmbeddedKeys(BTreeMap<String, String>),
}

#[derive(Clone)]
pub struct ArtifactCache {
    pub(crate) root: PathBuf,
    trust: TrustPolicy,
    offline: bool,
    pub(crate) download: DownloadOptions,
    url_override: Option<UrlOverride>,
    verified_stamp: bool,
}

impl std::fmt::Debug for ArtifactCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactCache")
            .field("root", &self.root)
            .field("trust", &self.trust)
            .field("offline", &self.offline)
            .field("download", &self.download)
            .field("url_override", &self.url_override.as_ref().map(|_| "<fn>"))
            .field("verified_stamp", &self.verified_stamp)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedArtifact {
    pub root: PathBuf,
    pub library: PathBuf,
    pub manifest: ComponentManifest,
}

impl ArtifactCache {
    pub fn new(root: impl Into<PathBuf>, trust: TrustPolicy) -> Self {
        Self {
            root: root.into(),
            trust,
            offline: false,
            download: DownloadOptions::default(),
            url_override: None,
            verified_stamp: false,
        }
    }

    /// Replace the download tuning (timeouts, retries, progress, cancellation).
    #[must_use]
    pub fn with_download_options(mut self, options: DownloadOptions) -> Self {
        self.download = options;
        self
    }

    #[must_use]
    pub fn download_options(&self) -> &DownloadOptions {
        &self.download
    }

    /// Fetch artifacts from `resolve(entry)` instead of the lock entry's own `url`.
    ///
    /// Intended for air-gapped or corporate mirrors. Integrity never depends on the URL: the
    /// archive size, SHA-256 and the manifest signature are still checked against the lock.
    /// A returned `file://` URL is read from disk.
    #[must_use]
    pub fn with_url_override(
        mut self,
        resolve: impl Fn(&ComponentLockEntry) -> String + Send + Sync + 'static,
    ) -> Self {
        self.url_override = Some(Arc::new(resolve));
        self
    }

    /// Trust a verified stamp instead of re-hashing an installed library on every process
    /// start. Off by default.
    ///
    /// After the first full check a stamp (archive sha, size, mtime in nanoseconds, and on Unix
    /// device, inode and ctime) is written next to the object. A later process that finds the
    /// library still matching the stamp skips reading and hashing it; the manifest digest,
    /// identity and signature are still verified every time.
    ///
    /// Still caught: replacing the file (new inode), size changes, and edits that change the
    /// mtime or, on Unix, the ctime. Not caught: a same-size in-place edit that restores the
    /// mtime on Windows, or by an attacker who can forge Unix ctime. Anyone able to do that can
    /// already rewrite the cache (including the stamp), so the stamp is honoured only while the
    /// cache root and its object directory are owned by the current user and not writable by
    /// group or others (Unix); otherwise every verification hashes the library. The cache creates
    /// its directories `0700`.
    #[must_use]
    pub fn with_verified_stamp(mut self, enabled: bool) -> Self {
        self.verified_stamp = enabled;
        self
    }

    fn resolve_url(&self, entry: &ComponentLockEntry) -> String {
        match &self.url_override {
            Some(resolve) => resolve(entry),
            None => entry.url.clone(),
        }
    }

    /// Disable network downloads: [`Self::install`] fails a missing artifact with
    /// [`ComponentError::Offline`] instead of attempting a request, and a damaged installed
    /// artifact with [`ComponentError::CachedObjectInvalid`]. An artifact already verified on
    /// disk (or served via a `file://` URL) is unaffected.
    #[must_use]
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn object_path(&self, entry: &ComponentLockEntry) -> Result<PathBuf, ComponentError> {
        decode_digest(&entry.sha256)?;
        Ok(self.root.join("sha256").join(&entry.sha256))
    }

    /// Verify an installed object in place: checks it against the lock entry (skipping the
    /// library hash when a current verified stamp says it is unchanged) but never downloads or
    /// removes anything, and writes nothing but the verified stamp when the cache was built with
    /// [`Self::with_verified_stamp(true)`].
    ///
    /// Unlike [`Self::install`], a failing check is reported instead of repaired, and the error
    /// says which check failed (digest, size, manifest, signature) -- so a caller can tell
    /// "installed and damaged" apart from "never installed" and log or report a repair.
    pub fn verify_cached(&self, entry: &ComponentLockEntry) -> Result<CachedArtifact, ComponentError> {
        self.verify_cached_file(entry).map(|(cached, _)| cached)
    }

    /// [`Self::verify_cached`], also returning the open, verified library, which is what a caller
    /// that loads or executes the artifact must use (see [`VerifiedFile`]).
    pub(crate) fn verify_cached_file(
        &self,
        entry: &ComponentLockEntry,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let destination = self.object_path(entry)?;
        self.verify_cached_at(entry, &destination)
    }

    pub fn install(&self, entry: &ComponentLockEntry) -> Result<CachedArtifact, ComponentError> {
        self.install_verified(entry).map(|(cached, _)| cached)
    }

    /// [`Self::install`], also returning the open, verified library (see [`VerifiedFile`]).
    pub(crate) fn install_verified(
        &self,
        entry: &ComponentLockEntry,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let destination = self.object_path(entry)?;
        let mut damaged = None;
        if destination.is_dir() {
            match self.verify_cached_at(entry, &destination) {
                Ok(verified) => {
                    self.touch_used(&entry.sha256);
                    return Ok(verified);
                }
                Err(error) => damaged = Some(error),
            }
        }
        let url = self.resolve_url(entry);
        if let Some(path) = url.strip_prefix("file://") {
            return self.install_reader_verified(entry, File::open(path)?);
        }
        if self.offline {
            return match damaged {
                Some(source) => Err(ComponentError::CachedObjectInvalid {
                    component: entry.identity.component.clone(),
                    source: Box::new(source),
                }),
                None => Err(ComponentError::Offline {
                    component: entry.identity.component.clone(),
                    url,
                }),
            };
        }
        let attempts = self.download.max_attempts.max(1);
        let mut attempt = 1;
        loop {
            if self.download.is_cancelled() {
                return Err(ComponentError::Cancelled);
            }
            let (error, transient) = match self.download_once(entry, &url) {
                Ok(verified) => return Ok(verified),
                Err(failure) => failure,
            };
            if !transient || attempt >= attempts {
                return Err(error);
            }
            std::thread::sleep(self.download.backoff_delay(attempt));
            attempt += 1;
        }
    }

    /// One request-and-install attempt; the flag says whether a failure is worth retrying.
    fn download_once(
        &self,
        entry: &ComponentLockEntry,
        url: &str,
    ) -> Result<(CachedArtifact, VerifiedFile), (ComponentError, bool)> {
        let tls = self.download.tls_config(url).map_err(|error| (error, false))?;
        let response = ureq::get(url)
            .config()
            .tls_config(tls)
            .timeout_connect(Some(self.download.connect_timeout))
            .timeout_global(Some(self.download.total_timeout))
            .build()
            .header(
                "User-Agent",
                concat!("alef-component-runtime/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| {
                let transient = is_transient_request_error(&error);
                (
                    ComponentError::Download {
                        url: url.to_owned(),
                        message: error.to_string(),
                    },
                    transient,
                )
            })?;
        let mut body = TrackedReader::new(response.into_body().into_reader());
        let result = self.install_reader_verified(entry, &mut body);
        result.map_err(|error| {
            let transient = body.read_failed && matches!(error, ComponentError::Io(_));
            (error, transient)
        })
    }

    /// Install an archive from `reader`, honouring this cache's progress and cancellation
    /// settings (see [`DownloadOptions`]). Never retries: the reader cannot be rewound.
    pub fn install_from_reader(
        &self,
        entry: &ComponentLockEntry,
        reader: impl Read,
    ) -> Result<CachedArtifact, ComponentError> {
        self.install_reader_verified(entry, reader).map(|(cached, _)| cached)
    }

    fn install_reader_verified(
        &self,
        entry: &ComponentLockEntry,
        reader: impl Read,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let verified = self.install_stream(entry, reader)?;
        self.touch_used(&entry.sha256);
        Ok(verified)
    }

    fn install_stream(
        &self,
        entry: &ComponentLockEntry,
        mut reader: impl Read,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let expected = decode_digest(&entry.sha256)?;
        let objects = self.root.join("sha256");
        create_private_dir_all(&objects)?;
        let lock = self.digest_lock_file(&entry.sha256)?;
        lock.lock()?;

        let destination = self.object_path(entry)?;
        if destination.is_dir()
            && let Ok(verified) = self.verify_cached_at(entry, &destination)
        {
            return Ok(verified);
        }

        let mut archive = tempfile::NamedTempFile::new_in(&objects)?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if self.download.is_cancelled() {
                return Err(ComponentError::Cancelled);
            }
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            archive.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            copied += count as u64;
            if copied > entry.size {
                return Err(ComponentError::SizeMismatch {
                    expected: entry.size,
                    actual: copied,
                });
            }
            self.download.report(copied, entry.size);
        }
        if entry.size != copied {
            return Err(ComponentError::SizeMismatch {
                expected: entry.size,
                actual: copied,
            });
        }
        let actual: [u8; 32] = hasher.finalize().into();
        if actual != expected {
            return Err(ComponentError::DigestMismatch {
                expected: entry.sha256.clone(),
                actual: hex::encode(actual),
            });
        }

        archive.as_file_mut().seek(SeekFrom::Start(0))?;
        let staging = tempfile::Builder::new().prefix(".install-").tempdir_in(&objects)?;
        extract_safely(archive.as_file_mut(), staging.path())?;
        let (cached, verified) = self.verify_installation(entry, staging.path(), None)?;

        if destination.exists() {
            fs::remove_dir_all(&destination)?;
        }
        let _ = fs::remove_file(self.stamp_path(&entry.sha256));
        let staging_path = staging.keep();
        if let Err(error) = fs::rename(&staging_path, &destination) {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(ComponentError::Io(error));
        }
        let library = destination.join(&cached.manifest.library.file);
        // A rename keeps the inode, so the descriptor verified in staging is the installed file.
        let verified = verified.relocated(library.clone());
        // Stamped from the verified descriptor, not the path: a file swapped in after the rename
        // must not inherit the stamp.
        if self.stamp_enabled()
            && let Ok(stamp) = verified
                .metadata()
                .and_then(|metadata| stamp::from_metadata(&entry.sha256, &metadata))
        {
            stamp::write(&self.stamp_path(&entry.sha256), &stamp);
        }
        Ok((
            CachedArtifact {
                root: destination.clone(),
                library,
                manifest: cached.manifest,
            },
            verified,
        ))
    }

    /// Open (creating it if needed) the per-digest lock file every install and removal of that
    /// object holds. Lock files are never deleted: unlinking one while another process waits on
    /// it would let a third process lock a fresh file at the same path, and two writers would
    /// then each believe they hold the object exclusively.
    pub(crate) fn digest_lock_file(&self, sha256: &str) -> std::io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.objects_dir().join(format!("{sha256}.lock")))
    }

    pub(crate) fn stamp_path(&self, sha256: &str) -> PathBuf {
        self.root.join("sha256").join(format!("{sha256}.verified"))
    }

    /// Verify an object already in the cache, using the verified stamp when enabled.
    fn verify_cached_at(
        &self,
        entry: &ComponentLockEntry,
        root: &Path,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let stamp_path = self.stamp_path(&entry.sha256);
        self.verify_installation(entry, root, self.stamp_enabled().then_some(stamp_path.as_path()))
    }

    /// Whether the verified stamp may be read and written: it was requested, and only the
    /// current user can write the cache. A stamp in a directory others can write vouches for
    /// nothing, so such a cache is fully re-verified every time instead (ADR-0026).
    fn stamp_enabled(&self) -> bool {
        self.verified_stamp && is_private_dir(&self.root) && is_private_dir(&self.objects_dir())
    }

    fn verify_installation(
        &self,
        entry: &ComponentLockEntry,
        root: &Path,
        stamp_path: Option<&Path>,
    ) -> Result<(CachedArtifact, VerifiedFile), ComponentError> {
        let manifest_bytes = fs::read(root.join("component.json"))?;
        if sha256(&manifest_bytes) != decode_digest(&entry.manifest_sha256)? {
            return Err(ComponentError::ManifestDigestMismatch);
        }
        let manifest: ComponentManifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.canonical_bytes()? != manifest_bytes {
            return Err(ComponentError::NonCanonicalManifest);
        }
        if manifest.schema_version != COMPONENT_MANIFEST_SCHEMA || manifest.abi_version != COMPONENT_ABI_VERSION {
            return Err(ComponentError::UnsupportedManifest);
        }
        if manifest.identity != entry.identity {
            return Err(ComponentError::ManifestIdentityMismatch);
        }
        let library_relative = safe_relative_path(&manifest.library.file)?;
        let library = root.join(&library_relative);
        let missing = |error: std::io::Error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ComponentError::MissingLibrary(library.clone())
            } else {
                ComponentError::Io(error)
            }
        };
        // Every check below reads the library through this one descriptor, and the caller loads
        // or executes that same file (see `identity`), so swapping the path afterwards cannot
        // substitute unverified code.
        let file = File::open(&library).map_err(missing)?;
        let metadata = file.metadata()?;
        if metadata.len() != manifest.library.size {
            return Err(ComponentError::SizeMismatch {
                expected: manifest.library.size,
                actual: metadata.len(),
            });
        }
        let trusted = stamp_path.is_some_and(|path| stamp::matches(path, &entry.sha256, &metadata));
        if !trusted {
            let expected_library = decode_digest(&manifest.library.sha256)?;
            // Bounded one byte past the declared size, so a file that grew after the metadata
            // check is caught as a size mismatch rather than hashed without limit.
            let (actual_library, hashed) = sha256_reader((&file).take(manifest.library.size.saturating_add(1)))?;
            if hashed != manifest.library.size {
                return Err(ComponentError::SizeMismatch {
                    expected: manifest.library.size,
                    actual: hashed,
                });
            }
            if expected_library != actual_library {
                return Err(ComponentError::DigestMismatch {
                    expected: manifest.library.sha256.clone(),
                    actual: hex::encode(actual_library),
                });
            }
        }
        self.verify_manifest_signature(entry, root, &manifest_bytes)?;
        // Record the stamp only when the file did not change while it was being hashed.
        if let Some(path) = stamp_path
            && !trusted
            && file
                .metadata()
                .is_ok_and(|after| FileIdentity::of(&after) == FileIdentity::of(&metadata))
            && let Ok(stamp) = stamp::from_metadata(&entry.sha256, &metadata)
        {
            stamp::write(path, &stamp);
        }
        let verified = VerifiedFile::new(library.clone(), file, &metadata);
        Ok((
            CachedArtifact {
                root: root.to_owned(),
                library,
                manifest,
            },
            verified,
        ))
    }

    fn verify_manifest_signature(
        &self,
        entry: &ComponentLockEntry,
        root: &Path,
        manifest_bytes: &[u8],
    ) -> Result<(), ComponentError> {
        let TrustPolicy::EmbeddedKeys(keys) = &self.trust else {
            return Ok(());
        };
        if entry.key_id.is_empty() && keys.is_empty() {
            return Ok(());
        }
        let signature_bytes = fs::read(root.join("component.sig")).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ComponentError::SignatureRequired
            } else {
                ComponentError::Io(error)
            }
        })?;
        let signature: ComponentSignature = serde_json::from_slice(&signature_bytes)?;
        if signature.algorithm != "ed25519" {
            return Err(ComponentError::UnsupportedSignatureAlgorithm(signature.algorithm));
        }
        if signature.key_id != entry.key_id {
            return Err(ComponentError::SignatureKeyMismatch);
        }
        let encoded_key = keys
            .get(&signature.key_id)
            .ok_or_else(|| ComponentError::UnknownSignatureKey(signature.key_id.clone()))?;
        let key = decode_public_key(encoded_key)?;
        let raw_signature = base64::engine::general_purpose::STANDARD
            .decode(&signature.signature)
            .map_err(|_| ComponentError::InvalidSignatureEncoding)?;
        let signature = Signature::from_slice(&raw_signature).map_err(|_| ComponentError::InvalidSignatureEncoding)?;
        key.verify(manifest_bytes, &signature)
            .map_err(|_| ComponentError::SignatureVerification)
    }
}

/// Create `path` and any missing parents readable and writable only by the current user. Existing
/// directories keep their permissions; [`is_private_dir`] decides what the cache trusts in them.
fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Whether `path` is a directory owned by the current user that neither its group nor others can
/// write. On non-Unix platforms permissions are ACLs this does not inspect, so it trusts the
/// per-user cache location (`%LOCALAPPDATA%`) the host chose.
pub(crate) fn is_private_dir(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        // SAFETY: `geteuid` takes no arguments, cannot fail and touches no memory.
        let user = unsafe { libc::geteuid() };
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.uid() == user && metadata.mode() & 0o022 == 0)
    }
    #[cfg(not(unix))]
    {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    }
}

pub(crate) fn decode_digest(value: &str) -> Result<[u8; 32], ComponentError> {
    let bytes = hex::decode(value).map_err(|_| ComponentError::InvalidDigest(value.to_owned()))?;
    bytes
        .try_into()
        .map_err(|_| ComponentError::InvalidDigest(value.to_owned()))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Size of the read buffer used to hash installed libraries.
const HASH_CHUNK: usize = 64 * 1024;

/// SHA-256 of everything `reader` yields, read in [`HASH_CHUNK`] pieces so a large library is
/// never held in memory whole. Returns the digest and the number of bytes hashed.
fn sha256_reader(mut reader: impl Read) -> std::io::Result<([u8; 32], u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK];
    let mut total = 0_u64;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..count]);
        total += count as u64;
    }
    Ok((hasher.finalize().into(), total))
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ComponentError> {
    let path = Path::new(value);
    let safe = !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if safe {
        Ok(path.to_owned())
    } else {
        Err(ComponentError::UnsafeLibraryPath(value.to_owned()))
    }
}

/// Generous ceiling for the manifest and signature entries: both are small JSON documents,
/// orders of magnitude smaller than any native component library, so a declared size above
/// this already means the archive is not one `alef component package` produced.
const METADATA_ENTRY_SIZE_CEILING: u64 = 64 * 1024;
/// Headroom added to the manifest's declared library size before an oversized decompressed
/// library entry is rejected, covering tar block padding around the real content.
const LIBRARY_SIZE_HEADROOM: u64 = 4096;
/// `alef component package` always writes at most three archive entries: the manifest, an
/// optional signature, and the library. Anything else is not an archive this loader made.
const MAX_ARCHIVE_ENTRIES: usize = 3;

/// Extract a downloaded component archive, bounding how much a corrupted or malicious
/// archive can force this to decompress, and rejecting anything that is not exactly the
/// shape `alef component package` produces.
///
/// The manifest must be the first entry (`component.json`) because its declared
/// `library.size` is what bounds the decompressed size of the library entry that follows --
/// without reading it first there is nothing to size the library entry's ceiling against,
/// and an attacker-supplied archive could otherwise reorder entries to smuggle an oversized
/// library past a check performed too late. Every entry must be a regular file at a safe
/// relative path (no absolute paths, `..`, or symlinks/hardlinks), and the total entry count
/// is capped so a maliciously padded archive cannot force unbounded header parsing either.
fn extract_safely(reader: impl Read, destination: &Path) -> Result<(), ComponentError> {
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries()?;

    let mut first = entries
        .next()
        .ok_or_else(|| ComponentError::UnsafeArchiveEntry("archive is empty".to_owned()))??;
    let first_path = first.path()?.into_owned();
    if first_path != Path::new("component.json") || !first.header().entry_type().is_file() {
        return Err(ComponentError::UnsafeArchiveEntry(first_path.display().to_string()));
    }
    let manifest_bytes = read_bounded_entry(&mut first, METADATA_ENTRY_SIZE_CEILING)?;
    fs::write(destination.join("component.json"), &manifest_bytes)?;
    let manifest: ComponentManifest = serde_json::from_slice(&manifest_bytes)?;
    let library_relative = safe_relative_path(&manifest.library.file)?;
    let library_ceiling = manifest.library.size.saturating_add(LIBRARY_SIZE_HEADROOM);

    let mut seen_library = false;
    let mut entry_count = 1_usize;
    for entry in entries {
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            return Err(ComponentError::TooManyArchiveEntries {
                max: MAX_ARCHIVE_ENTRIES,
                actual: entry_count,
            });
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let is_safe_relative = !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !is_safe_relative || !entry.header().entry_type().is_file() {
            return Err(ComponentError::UnsafeArchiveEntry(path.display().to_string()));
        }
        if path == Path::new("component.sig") {
            let bytes = read_bounded_entry(&mut entry, METADATA_ENTRY_SIZE_CEILING)?;
            fs::write(destination.join("component.sig"), bytes)?;
        } else if path == library_relative && !seen_library {
            seen_library = true;
            stream_library_entry(&mut entry, destination, &library_relative, library_ceiling, &manifest)?;
        } else {
            return Err(ComponentError::UnsafeArchiveEntry(path.display().to_string()));
        }
    }
    if !seen_library {
        return Err(ComponentError::MissingLibrary(destination.join(&library_relative)));
    }
    Ok(())
}

/// Read a small, fully-buffered entry (the manifest or its signature), rejecting it before
/// any bytes are copied if its declared size already exceeds `ceiling`.
fn read_bounded_entry<R: Read>(entry: &mut tar::Entry<'_, R>, ceiling: u64) -> Result<Vec<u8>, ComponentError> {
    let declared = entry.header().size()?;
    if declared > ceiling {
        return Err(ComponentError::ExtractedSizeExceeded {
            limit: ceiling,
            actual: declared,
        });
    }
    let mut buffer = Vec::new();
    entry.read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// Stream the library entry straight to disk in fixed-size chunks -- rather than buffering
/// the whole (potentially large) decompressed library in memory -- hashing it as it is
/// written so the digest is verified in the same pass instead of a second read from disk.
fn stream_library_entry<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    destination: &Path,
    relative: &Path,
    ceiling: u64,
    manifest: &ComponentManifest,
) -> Result<(), ComponentError> {
    let declared = entry.header().size()?;
    if declared > ceiling {
        return Err(ComponentError::ExtractedSizeExceeded {
            limit: ceiling,
            actual: declared,
        });
    }
    let target = destination.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(&target)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let count = entry.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > ceiling {
            return Err(ComponentError::ExtractedSizeExceeded {
                limit: ceiling,
                actual: copied,
            });
        }
        hasher.update(&buffer[..count]);
        file.write_all(&buffer[..count])?;
    }
    if copied != manifest.library.size {
        return Err(ComponentError::SizeMismatch {
            expected: manifest.library.size,
            actual: copied,
        });
    }
    let actual: [u8; 32] = hasher.finalize().into();
    let expected = decode_digest(&manifest.library.sha256)?;
    if actual != expected {
        return Err(ComponentError::DigestMismatch {
            expected: manifest.library.sha256.clone(),
            actual: hex::encode(actual),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
