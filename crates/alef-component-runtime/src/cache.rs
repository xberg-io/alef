use crate::download::{DownloadOptions, TrackedReader, is_transient_request_error};
use crate::keys::decode_public_key;
use crate::manifest::{COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA};
use crate::stamp;
use crate::{ComponentError, ComponentLockEntry, ComponentManifest, ComponentSignature};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier as _};
use flate2::read::GzDecoder;
use fs2::FileExt as _;
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
    /// already rewrite the cache (including the stamp), so enable this only for caches that
    /// only the current user can write.
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
    /// [`ComponentError::Offline`] instead of attempting a request. An artifact already
    /// verified on disk (or served via a `file://` URL) is unaffected.
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

    pub fn install(&self, entry: &ComponentLockEntry) -> Result<CachedArtifact, ComponentError> {
        let destination = self.object_path(entry)?;
        if destination.is_dir()
            && let Ok(cached) = self.verify_cached(entry, &destination)
        {
            self.touch_used(&entry.sha256);
            return Ok(cached);
        }
        let url = self.resolve_url(entry);
        if let Some(path) = url.strip_prefix("file://") {
            return self.install_from_reader(entry, File::open(path)?);
        }
        if self.offline {
            return Err(ComponentError::Offline {
                component: entry.identity.component.clone(),
                url,
            });
        }
        let attempts = self.download.max_attempts.max(1);
        let mut attempt = 1;
        loop {
            if self.download.is_cancelled() {
                return Err(ComponentError::Cancelled);
            }
            let (error, transient) = match self.download_once(entry, &url) {
                Ok(cached) => return Ok(cached),
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
    fn download_once(&self, entry: &ComponentLockEntry, url: &str) -> Result<CachedArtifact, (ComponentError, bool)> {
        let response = ureq::get(url)
            .config()
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
        let result = self.install_from_reader(entry, &mut body);
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
        let cached = self.install_stream(entry, reader)?;
        self.touch_used(&entry.sha256);
        Ok(cached)
    }

    fn install_stream(
        &self,
        entry: &ComponentLockEntry,
        mut reader: impl Read,
    ) -> Result<CachedArtifact, ComponentError> {
        let expected = decode_digest(&entry.sha256)?;
        let objects = self.root.join("sha256");
        fs::create_dir_all(&objects)?;
        let lock_path = objects.join(format!("{}.lock", entry.sha256));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;

        let destination = self.object_path(entry)?;
        if destination.is_dir()
            && let Ok(cached) = self.verify_cached(entry, &destination)
        {
            return Ok(cached);
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
        let cached = self.verify_installation(entry, staging.path(), None)?;

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
        if self.verified_stamp
            && let Ok(stamp) = stamp::capture(&entry.sha256, &library)
        {
            stamp::write(&self.stamp_path(&entry.sha256), &stamp);
        }
        Ok(CachedArtifact {
            root: destination.clone(),
            library,
            manifest: cached.manifest,
        })
    }

    pub(crate) fn stamp_path(&self, sha256: &str) -> PathBuf {
        self.root.join("sha256").join(format!("{sha256}.verified"))
    }

    /// Verify an object already in the cache, using the verified stamp when enabled.
    fn verify_cached(&self, entry: &ComponentLockEntry, root: &Path) -> Result<CachedArtifact, ComponentError> {
        let stamp_path = self.stamp_path(&entry.sha256);
        self.verify_installation(entry, root, self.verified_stamp.then_some(stamp_path.as_path()))
    }

    fn verify_installation(
        &self,
        entry: &ComponentLockEntry,
        root: &Path,
        stamp_path: Option<&Path>,
    ) -> Result<CachedArtifact, ComponentError> {
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
        let trusted = stamp_path.is_some_and(|path| stamp::is_current(path, &entry.sha256, &library));
        let before = if trusted || stamp_path.is_none() {
            None
        } else {
            stamp::capture(&entry.sha256, &library).ok()
        };
        let missing = |error: std::io::Error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ComponentError::MissingLibrary(library.clone())
            } else {
                ComponentError::Io(error)
            }
        };
        if trusted {
            let size = fs::metadata(&library).map_err(missing)?.len();
            if size != manifest.library.size {
                return Err(ComponentError::SizeMismatch {
                    expected: manifest.library.size,
                    actual: size,
                });
            }
        } else {
            let library_bytes = fs::read(&library).map_err(missing)?;
            if library_bytes.len() as u64 != manifest.library.size {
                return Err(ComponentError::SizeMismatch {
                    expected: manifest.library.size,
                    actual: library_bytes.len() as u64,
                });
            }
            let expected_library = decode_digest(&manifest.library.sha256)?;
            let actual_library = sha256(&library_bytes);
            if expected_library != actual_library {
                return Err(ComponentError::DigestMismatch {
                    expected: manifest.library.sha256.clone(),
                    actual: hex::encode(actual_library),
                });
            }
        }
        self.verify_manifest_signature(entry, root, &manifest_bytes)?;
        // Record the stamp only when the file did not change while it was being hashed.
        if let (Some(path), Some(before)) = (stamp_path, before)
            && stamp::capture(&entry.sha256, &library).is_ok_and(|after| after == before)
        {
            stamp::write(path, &before);
        }
        Ok(CachedArtifact {
            root: root.to_owned(),
            library,
            manifest,
        })
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

pub(crate) fn decode_digest(value: &str) -> Result<[u8; 32], ComponentError> {
    let bytes = hex::decode(value).map_err(|_| ComponentError::InvalidDigest(value.to_owned()))?;
    bytes
        .try_into()
        .map_err(|_| ComponentError::InvalidDigest(value.to_owned()))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
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
