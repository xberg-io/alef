use crate::loader::{decode_hash, validate_manifest};
use crate::manifest::COMPONENT_MANIFEST_SCHEMA;
use crate::{
    ArtifactCache, CachedArtifact, ComponentDeliveryMode, ComponentError, ComponentLock, ComponentLockEntry,
    ComponentRequirements, LoadedComponent, TrustPolicy,
};
use alef_component_abi::AlefHostApiV1;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentStatus {
    Missing,
    Cached(PathBuf),
    Loaded(PathBuf),
}

pub struct ComponentManager {
    cache: ArtifactCache,
    target: String,
    entries: HashMap<String, ComponentLockEntry>,
    loaded: Mutex<HashMap<String, Arc<LoadedComponent>>>,
    host: AlefHostApiV1,
}

// SAFETY: `LoadedComponent` is thread-safe metadata and the host callback
// context is required by the ABI to remain valid for calls from component
// threads. Mutation is protected by the manager mutex. ~keep
unsafe impl Send for ComponentManager {}
unsafe impl Sync for ComponentManager {}

impl ComponentManager {
    pub fn from_lock(
        lock: ComponentLock,
        cache_root: impl Into<PathBuf>,
        target: impl Into<String>,
        host: AlefHostApiV1,
    ) -> Result<Self, ComponentError> {
        if lock.schema_version != COMPONENT_MANIFEST_SCHEMA {
            return Err(ComponentError::UnsupportedLockSchema(lock.schema_version));
        }
        let target = target.into();
        let mut entries = HashMap::new();
        for artifact in lock
            .artifacts
            .into_iter()
            .filter(|artifact| artifact.identity.target == target)
        {
            let id = artifact.identity.component.clone();
            if entries.insert(id.clone(), artifact).is_some() {
                return Err(ComponentError::DuplicateLockEntry {
                    component_id: id,
                    target,
                });
            }
        }
        Ok(Self {
            cache: ArtifactCache::new(cache_root, TrustPolicy::EmbeddedKeys(lock.public_keys)),
            target,
            entries,
            loaded: Mutex::new(HashMap::new()),
            host,
        })
    }

    /// Load one contract a component provides.
    ///
    /// `entry_symbol` is that contract's NUL-terminated producer entry-point
    /// symbol (`alef::codegen::component::entry_point_symbol`); the caller
    /// supplies it because computing it requires the same identifier-casing
    /// logic the producer used, which this crate does not depend on.
    pub fn ensure_contract(
        &self,
        component_id: &str,
        contract_name: &str,
        entry_symbol: &[u8],
    ) -> Result<Arc<LoadedComponent>, ComponentError> {
        let cache_key = loaded_cache_key(component_id, contract_name);
        let mut loaded = self.loaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(component) = loaded.get(&cache_key) {
            return Ok(Arc::clone(component));
        }
        let entry = self.entry(component_id)?;
        let requirements = contract_requirements(entry, contract_name)?;
        let cached = self.cache.install(entry)?;
        validate_manifest(&cached.manifest, &requirements)?;
        let component = Arc::new(LoadedComponent::load(
            cached.library,
            entry_symbol,
            &requirements,
            self.host,
        )?);
        loaded.insert(cache_key, Arc::clone(&component));
        Ok(component)
    }

    pub fn prefetch(&self, component_ids: &[&str]) -> Result<Vec<CachedArtifact>, ComponentError> {
        component_ids
            .iter()
            .map(|component_id| self.cache.install(self.entry(component_id)?))
            .collect()
    }

    pub fn status(&self, component_id: &str) -> Result<ComponentStatus, ComponentError> {
        let root = self.cache.object_path(self.entry(component_id)?)?;
        let loaded_prefix = format!("{component_id}::");
        if self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .any(|key| key.starts_with(&loaded_prefix))
        {
            return Ok(ComponentStatus::Loaded(root));
        }
        if root.join("component.json").is_file() {
            Ok(ComponentStatus::Cached(root))
        } else {
            Ok(ComponentStatus::Missing)
        }
    }

    pub fn cache_path(&self, component_id: &str) -> Result<PathBuf, ComponentError> {
        self.cache.object_path(self.entry(component_id)?)
    }

    #[must_use]
    pub fn cache_root(&self) -> &Path {
        self.cache.root()
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Look up a component's lock entry for this manager's target.
    ///
    /// Every caller (`ensure`, `prefetch`, `status`, `cache_path`) reaches the
    /// cache only through this method, so a `Bundled` entry -- one this manager
    /// cannot download -- is rejected here, before any cache I/O runs.
    fn entry(&self, component_id: &str) -> Result<&ComponentLockEntry, ComponentError> {
        let entry = self
            .entries
            .get(component_id)
            .ok_or_else(|| ComponentError::ArtifactNotFound {
                component_id: component_id.to_owned(),
                target: self.target.clone(),
            })?;
        if entry.mode == ComponentDeliveryMode::Bundled {
            return Err(ComponentError::BundledComponentNotLoadable {
                component: component_id.to_owned(),
            });
        }
        Ok(entry)
    }
}

fn loaded_cache_key(component_id: &str, contract_name: &str) -> String {
    format!("{component_id}::{contract_name}")
}

fn contract_requirements(
    entry: &ComponentLockEntry,
    contract_name: &str,
) -> Result<ComponentRequirements, ComponentError> {
    let provided = entry
        .provides
        .iter()
        .find(|provided| provided.contract == contract_name)
        .ok_or_else(|| ComponentError::ContractNotProvided {
            component_id: entry.identity.component.clone(),
            contract: contract_name.to_owned(),
        })?;
    Ok(ComponentRequirements {
        component_id: entry.identity.component.clone(),
        contract_name: contract_name.to_owned(),
        contract_hash: decode_hash(&provided.contract_hash)?,
        feature_set_hash: Some(decode_hash(&entry.identity.feature_hash)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ComponentIdentity;
    use std::collections::BTreeMap;
    use std::ffi::c_void;

    fn host() -> AlefHostApiV1 {
        AlefHostApiV1 {
            struct_size: 0,
            abi_major: 0,
            abi_minor: 0,
            context: core::ptr::null_mut::<c_void>(),
            log: None,
        }
    }

    fn lock(target: &str) -> ComponentLock {
        lock_with_mode(target, ComponentDeliveryMode::Download)
    }

    fn lock_with_mode(target: &str, mode: ComponentDeliveryMode) -> ComponentLock {
        ComponentLock {
            schema_version: COMPONENT_MANIFEST_SCHEMA,
            public_keys: BTreeMap::new(),
            artifacts: vec![ComponentLockEntry {
                identity: ComponentIdentity {
                    crate_name: "demo-core".into(),
                    component: "demo".into(),
                    version: "1.0.0".into(),
                    target: target.into(),
                    feature_hash: hex::encode([2; 32]),
                    contract_hash: hex::encode([1; 32]),
                },
                provides: vec![crate::ComponentProvidedContract {
                    contract: "engine".into(),
                    interface_version: 1,
                    contract_hash: hex::encode([1; 32]),
                    implementation: "demo::Demo".into(),
                }],
                mode,
                url: "file:///does/not/exist".into(),
                sha256: hex::encode([3; 32]),
                size: 0,
                manifest_sha256: hex::encode([4; 32]),
                key_id: String::new(),
            }],
        }
    }

    #[test]
    fn manager_filters_lock_to_exact_target() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-b", host()).unwrap();
        assert!(matches!(
            manager.status("demo"),
            Err(ComponentError::ArtifactNotFound { .. })
        ));
    }

    #[test]
    fn manager_reports_content_address_before_download() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
        assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Missing);
        assert!(
            manager
                .cache_path("demo")
                .unwrap()
                .ends_with(format!("sha256/{}", hex::encode([3; 32])))
        );
    }

    #[test]
    fn duplicate_target_entry_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut lock = lock("target-a");
        lock.artifacts.push(lock.artifacts[0].clone());
        assert!(matches!(
            ComponentManager::from_lock(lock, dir.path(), "target-a", host()),
            Err(ComponentError::DuplicateLockEntry { .. })
        ));
    }

    #[test]
    fn ensure_contract_rejects_a_contract_the_component_does_not_provide() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
        assert!(matches!(
            manager.ensure_contract("demo", "missing-contract", b"irrelevant\0"),
            Err(ComponentError::ContractNotProvided { .. })
        ));
    }

    #[test]
    fn ensure_contract_reaches_the_download_step_for_a_provided_contract() {
        let dir = tempfile::tempdir().unwrap();
        let mut two_contracts = lock("target-a");
        two_contracts.artifacts[0].provides.push(crate::ComponentProvidedContract {
            contract: "second".into(),
            interface_version: 1,
            contract_hash: hex::encode([9; 32]),
            implementation: "demo::Second".into(),
        });
        let manager = ComponentManager::from_lock(two_contracts, dir.path(), "target-a", host()).unwrap();
        // Fails to find the library at the fake `file://` URL -- proving contract resolution
        // for "second" succeeded and the manager reached the download step, not that the
        // download itself succeeded.
        assert!(matches!(
            manager.ensure_contract("demo", "second", b"alef_component_entry_v1_second\0"),
            Err(ComponentError::Io(_))
        ));
    }

    #[test]
    fn bundled_component_is_rejected_before_touching_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ComponentManager::from_lock(
            lock_with_mode("target-a", ComponentDeliveryMode::Bundled),
            dir.path(),
            "target-a",
            host(),
        )
        .unwrap();

        assert!(matches!(
            manager.ensure_contract("demo", "engine", b"alef_component_entry_v1_engine\0"),
            Err(ComponentError::BundledComponentNotLoadable { .. })
        ));
        assert!(matches!(
            manager.status("demo"),
            Err(ComponentError::BundledComponentNotLoadable { .. })
        ));
        assert!(matches!(
            manager.prefetch(&["demo"]),
            Err(ComponentError::BundledComponentNotLoadable { .. })
        ));
        assert!(matches!(
            manager.cache_path("demo"),
            Err(ComponentError::BundledComponentNotLoadable { .. })
        ));
        assert!(
            std::fs::read_dir(dir.path()).unwrap().next().is_none(),
            "a bundled component must never touch the cache directory"
        );
    }
}
