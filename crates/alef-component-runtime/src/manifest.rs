use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const COMPONENT_MANIFEST_SCHEMA: u32 = 2;
pub const COMPONENT_ABI_VERSION: u32 = 1;

/// One contract a component provides, alongside the Rust type implementing it.
///
/// A component may provide more than one contract (one producer entrypoint per
/// contract); each entry keeps its own `contract_hash` so a change to any single
/// contract's ABI is independently detectable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentProvidedContract {
    pub contract: String,
    pub interface_version: u32,
    pub contract_hash: String,
    pub implementation: String,
}

/// Whether a component is downloaded on demand or must be linked into the binding
/// ahead of time. `alef`'s config-resolution code (`alef::codegen::component`) re-exports
/// this type and exposes it on the resolved component config as a forward-looking
/// hook; no loader or backend acts on `Bundled` yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentDeliveryMode {
    Download,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentIdentity {
    pub crate_name: String,
    pub component: String,
    pub version: String,
    pub target: String,
    pub feature_hash: String,
    /// Hash of the first (lexicographically, by contract name) contract this
    /// component provides. For a component providing exactly one contract --
    /// the only shape the v1 loader fully validates today -- this is that
    /// contract's real hash. See [`ComponentManifest::provides`] for the
    /// complete, per-contract list.
    pub contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentLibrary {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub schema_version: u32,
    pub abi_version: u32,
    pub identity: ComponentIdentity,
    /// Every contract this component provides, sorted by contract name.
    pub provides: Vec<ComponentProvidedContract>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub library: ComponentLibrary,
}

impl ComponentManifest {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        canonical_json(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSignature {
    pub algorithm: String,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentArtifactRecord {
    pub manifest: ComponentManifest,
    pub manifest_sha256: String,
    pub archive: String,
    pub archive_sha256: String,
    pub archive_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<ComponentSignature>,
}

impl ComponentArtifactRecord {
    #[must_use]
    pub fn record_path(&self, output_dir: &Path) -> PathBuf {
        output_dir.join(format!("{}.record.json", self.archive))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentLock {
    pub schema_version: u32,
    pub public_keys: BTreeMap<String, String>,
    pub artifacts: Vec<ComponentLockEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentLockEntry {
    pub identity: ComponentIdentity,
    /// Every contract this component provides, mirrored from the manifest so
    /// a consumer can inspect them without downloading and parsing the archive.
    pub provides: Vec<ComponentProvidedContract>,
    /// Whether `identity.target` downloads this artifact or must link it into the
    /// binding. Every entry produced by `alef component lock` today is `Download`;
    /// no backend builds or locks a `Bundled` artifact yet.
    pub mode: ComponentDeliveryMode,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub manifest_sha256: String,
    pub key_id: String,
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_is_compact_and_newline_terminated() {
        let identity = ComponentIdentity {
            crate_name: "demo-core".into(),
            component: "fast".into(),
            version: "1.0.0".into(),
            target: "aarch64-apple-darwin".into(),
            feature_hash: "11".repeat(32),
            contract_hash: "22".repeat(32),
        };
        let bytes = canonical_json(&identity).unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.contains(&b' '));
    }

    #[test]
    fn lock_rejects_unknown_fields() {
        let json = r#"{"schema_version":1,"public_keys":{},"artifacts":[],"extra":true}"#;
        assert!(serde_json::from_str::<ComponentLock>(json).is_err());
    }
}
