//! Verified artifact installation and dynamic loading for Alef native components.
//!
//! # Downloads
//!
//! [`DownloadOptions`] tunes timeouts, retries (transient errors only), progress reporting and
//! cancellation; pass it to [`ComponentManager::with_download_options`] or
//! [`ArtifactCache::with_download_options`]. The defaults match the historical 10 s connect and
//! 300 s total timeouts.
//!
//! # Root certificates
//!
//! Downloads trust the compiled-in Mozilla roots ([`RootCerts::WebPki`]). Behind a TLS-inspecting
//! proxy, enable this crate's `platform-verifier` feature and select
//! [`RootCerts::PlatformVerifier`] with [`DownloadOptions::with_root_certs`] to use the operating
//! system trust store. Requesting it without the feature fails the download with a clear error.
//!
//! # Mirrors
//!
//! A lock entry's `url` is fixed when the lock is generated. Air-gapped and corporate hosts can
//! redirect downloads at runtime with [`ComponentManager::with_url_override`] (any
//! `Fn(&ComponentLockEntry) -> String`) or [`ComponentManager::with_mirror_base`] (serve every
//! archive as `{base}/{archive file name}`). Integrity does not depend on the URL: the archive
//! size and SHA-256 and the manifest signature are checked against the lock regardless of where
//! the bytes came from, so a mirror cannot substitute content.
//!
//! # Cache maintenance
//!
//! [`ComponentManager::list_cached`], [`ComponentManager::remove`] and
//! [`ComponentManager::prune`] (with [`Retain`]) manage the shared on-disk cache. Loading an
//! object touches a last-used stamp so several host versions can share one cache and prune by
//! recency. Prune also sweeps crash leftovers (`.install-*` staging dirs, orphaned temp files and
//! stale `.lock` files). Files Windows refuses to delete because a library is loaded are listed
//! under [`RemovalReport::skipped`].
//!
//! # Verified stamp
//!
//! [`ComponentManager::with_verified_stamp`] (off by default) skips re-hashing an installed
//! library when a stamp recorded after a full check still matches the file's size, mtime and, on
//! Unix, device, inode and ctime. See [`ArtifactCache::with_verified_stamp`] for what this does
//! and does not protect against.

mod cache;
mod download;
mod error;
mod host_services;
mod keys;
mod loader;
mod maintenance;
mod manager;
mod manifest;
mod stamp;
pub mod stdio;
pub mod target;

pub use cache::{ArtifactCache, CachedArtifact, TrustPolicy};
pub use download::{DownloadOptions, ProgressCallback, RootCerts};
pub use error::ComponentError;
pub use host_services::HostServices;
pub use keys::decode_public_key;
pub use loader::{
    ComponentInstance, ComponentRequirements, LoadedComponent, Runtime, take_owned_bytes, take_owned_text,
};
pub use maintenance::{CachedObject, RemovalReport, Retain, SkippedRemoval};
pub use manager::{ComponentManager, ComponentStatus};
pub use manifest::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentDeliveryMode,
    ComponentIdentity, ComponentLibrary, ComponentLock, ComponentLockEntry, ComponentManifest,
    ComponentProvidedContract, ComponentSignature, canonical_json,
};
pub use stdio::{SidecarRestartEvent, SidecarSession, SidecarSessionSetup};

pub use ed25519_dalek::VerifyingKey;
