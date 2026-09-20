//! Verified artifact installation and dynamic loading for Alef native components.

mod cache;
mod error;
mod keys;
mod loader;
mod manager;
mod manifest;

pub use cache::{ArtifactCache, CachedArtifact, TrustPolicy};
pub use error::ComponentError;
pub use keys::decode_public_key;
pub use loader::{
    ComponentInstance, ComponentRequirements, LoadedComponent, Runtime, take_owned_bytes, take_owned_text,
};
pub use manager::{ComponentManager, ComponentStatus};
pub use manifest::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentDeliveryMode,
    ComponentIdentity, ComponentLibrary, ComponentLock, ComponentLockEntry, ComponentManifest,
    ComponentProvidedContract, ComponentSignature, canonical_json,
};

pub use ed25519_dalek::VerifyingKey;
