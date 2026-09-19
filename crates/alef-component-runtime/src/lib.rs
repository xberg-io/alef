//! Verified artifact installation and dynamic loading for Alef native components.

mod cache;
mod error;
mod keys;
mod loader;
mod manager;
mod manifest;

pub use cache::{ArtifactCache, CachedArtifact, TrustPolicy};
pub use error::ComponentError;
pub use keys::{decode_public_key, encode_public_key_der};
pub use loader::{ComponentInstance, ComponentRequirements, LoadedComponent, Runtime};
pub use manager::{ComponentManager, ComponentStatus};
pub use manifest::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentIdentity, ComponentLibrary,
    ComponentLock, ComponentLockEntry, ComponentManifest, ComponentSignature, canonical_json,
};

pub use ed25519_dalek::VerifyingKey;
