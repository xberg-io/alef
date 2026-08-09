use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const COMPONENT_MANIFEST_SCHEMA: u32 = 1;
pub const COMPONENT_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentIdentity {
    pub crate_name: String,
    pub component: String,
    pub version: String,
    pub target: String,
    pub feature_hash: String,
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
    pub contract: String,
    pub contract_version: u32,
    pub implementation: String,
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
