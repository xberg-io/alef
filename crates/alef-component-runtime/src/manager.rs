use crate::loader::{decode_hash, validate_manifest};
use crate::{
    ArtifactCache, CachedArtifact, ComponentError, ComponentLock, ComponentLockEntry, ComponentRequirements,
    LoadedComponent, TrustPolicy,
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
        if lock.schema_version != 1 {
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

    pub fn ensure(&self, component_id: &str) -> Result<Arc<LoadedComponent>, ComponentError> {
        let mut loaded = self.loaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(component) = loaded.get(component_id) {
            return Ok(Arc::clone(component));
        }
        let entry = self.entry(component_id)?;
        let requirements = requirements(entry)?;
        let cached = self.cache.install(entry)?;
        validate_manifest(&cached.manifest, &requirements)?;
        let component = Arc::new(LoadedComponent::load(cached.library, &requirements, self.host)?);
        loaded.insert(component_id.to_owned(), Arc::clone(&component));
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
        if self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(component_id)
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

    fn entry(&self, component_id: &str) -> Result<&ComponentLockEntry, ComponentError> {
        self.entries
            .get(component_id)
            .ok_or_else(|| ComponentError::ArtifactNotFound {
                component_id: component_id.to_owned(),
                target: self.target.clone(),
            })
    }
}

fn requirements(entry: &ComponentLockEntry) -> Result<ComponentRequirements, ComponentError> {
    Ok(ComponentRequirements {
        component_id: entry.identity.component.clone(),
        contract_hash: decode_hash(&entry.identity.contract_hash)?,
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
        ComponentLock {
            schema_version: 1,
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
}
