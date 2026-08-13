use crate::{
    ArtifactCache, COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentError, ComponentLockEntry,
    ComponentManifest,
};
use alef_component_abi::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AlefComponentEntryV1, AlefComponentV1, AlefContract, AlefHostApiV1, AlefOwnedBuffer,
    COMPONENT_ENTRYPOINT_V1,
};
use libloading::Library;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

const MAX_DESCRIPTOR_STRING: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct ComponentRequirements {
    pub component_id: String,
    pub contract_hash: [u8; 32],
    pub feature_set_hash: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub struct Runtime {
    cache: ArtifactCache,
}

impl Runtime {
    pub fn new(cache: ArtifactCache) -> Self {
        Self { cache }
    }

    pub fn install_and_load(
        &self,
        entry: &ComponentLockEntry,
        requirements: &ComponentRequirements,
        host: AlefHostApiV1,
    ) -> Result<LoadedComponent, ComponentError> {
        let cached = self.cache.install(entry)?;
        validate_manifest(&cached.manifest, requirements)?;
        LoadedComponent::load(cached.library, requirements, host)
    }
}

pub struct LoadedComponent {
    library: Arc<Library>,
    descriptor: NonNull<AlefComponentV1>,
    host: Box<AlefHostApiV1>,
    component_id: String,
    component_version: String,
}

/// A live instance created by a component, paired with its typed C function table.
///
/// Dropping the instance invokes the component's own destructor while the
/// originating dynamic library remains pinned.
pub struct ComponentInstance<T: AlefContract> {
    component: Arc<LoadedComponent>,
    handle: NonNull<c_void>,
    _contract: PhantomData<T>,
}

// SAFETY: the descriptor is static component metadata and the library is pinned
// for the process lifetime. Host callback context thread-safety is part of the
// `AlefHostApiV1` contract supplied by the caller. ~keep
unsafe impl Send for LoadedComponent {}
unsafe impl Sync for LoadedComponent {}

impl LoadedComponent {
    pub fn load(
        path: impl Into<std::path::PathBuf>,
        requirements: &ComponentRequirements,
        mut host: AlefHostApiV1,
    ) -> Result<Self, ComponentError> {
        let path = path.into();
        host.struct_size = core::mem::size_of::<AlefHostApiV1>();
        host.abi_major = ABI_MAJOR_V1;
        host.abi_minor = ABI_MINOR_V1;
        let host = Box::new(host);

        let library = Arc::new(
            unsafe { Library::new(&path) }.map_err(|error| ComponentError::LibraryLoad {
                path: path.clone(),
                message: error.to_string(),
            })?,
        );
        let descriptor = unsafe {
            let entry: libloading::Symbol<'_, AlefComponentEntryV1> = library
                .get(COMPONENT_ENTRYPOINT_V1)
                .map_err(|error| ComponentError::MissingEntrypoint(error.to_string()))?;
            let mut raw = core::ptr::null();
            let status = entry(ABI_MAJOR_V1, host.as_ref(), &mut raw);
            if !status.is_ok() {
                return Err(ComponentError::EntrypointFailed(status.0));
            }
            NonNull::new(raw.cast_mut()).ok_or(ComponentError::InvalidDescriptor("null descriptor"))?
        };

        let validated = unsafe { validate_descriptor(descriptor.as_ref(), requirements)? };
        pin_for_process(Arc::clone(&library));
        Ok(Self {
            library,
            descriptor,
            host,
            component_id: validated.0,
            component_version: validated.1,
        })
    }

    #[must_use]
    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    #[must_use]
    pub fn component_version(&self) -> &str {
        &self.component_version
    }

    #[must_use]
    pub fn descriptor(&self) -> &AlefComponentV1 {
        unsafe { self.descriptor.as_ref() }
    }

    #[must_use]
    pub fn contract(&self) -> *const c_void {
        self.descriptor().contract
    }

    #[must_use]
    pub fn contract_size(&self) -> usize {
        self.descriptor().contract_size
    }

    pub fn contract_table<T: AlefContract>(&self) -> Result<&T, ComponentError> {
        contract_table_from_descriptor(self.descriptor())
    }

    /// Create an instance and bind it to a generated contract-table type.
    pub fn instantiate<T: AlefContract>(self: &Arc<Self>) -> Result<ComponentInstance<T>, ComponentError> {
        self.contract_table::<T>()?;
        let create = self.descriptor().create.ok_or(ComponentError::MissingInstanceFactory)?;
        self.descriptor()
            .destroy
            .ok_or(ComponentError::MissingInstanceDestructor)?;
        let mut raw = core::ptr::null_mut();
        let mut error = AlefOwnedBuffer::EMPTY;
        let status = unsafe { create(self.host_api(), &mut raw, &mut error) };
        if !status.is_ok() {
            return Err(ComponentError::InstanceCreation {
                status: status.0,
                message: unsafe { take_owned_buffer(error) },
            });
        }
        let handle = NonNull::new(raw).ok_or(ComponentError::NullInstance)?;
        Ok(ComponentInstance {
            component: Arc::clone(self),
            handle,
            _contract: PhantomData,
        })
    }

    #[must_use]
    pub fn host_api(&self) -> &AlefHostApiV1 {
        &self.host
    }

    #[must_use]
    pub fn strong_library_references(&self) -> usize {
        Arc::strong_count(&self.library)
    }
}

impl<T: AlefContract> ComponentInstance<T> {
    #[must_use]
    pub fn table(&self) -> &T {
        // The table was validated before this instance was constructed.
        self.component
            .contract_table::<T>()
            .expect("validated component contract table changed")
    }

    #[must_use]
    pub fn handle(&self) -> *mut c_void {
        self.handle.as_ptr()
    }

    #[must_use]
    pub fn component(&self) -> &Arc<LoadedComponent> {
        &self.component
    }
}

impl<T: AlefContract> Drop for ComponentInstance<T> {
    fn drop(&mut self) {
        if let Some(destroy) = self.component.descriptor().destroy {
            unsafe { destroy(self.handle.as_ptr()) };
        }
    }
}

unsafe fn take_owned_buffer(buffer: AlefOwnedBuffer) -> String {
    let message = if buffer.ptr.is_null() {
        if buffer.len == 0 {
            String::new()
        } else {
            "component returned an invalid error buffer".to_string()
        }
    } else {
        String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(buffer.ptr, buffer.len) }).into_owned()
    };
    if let Some(free) = buffer.free {
        unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
    }
    message
}

fn contract_table_from_descriptor<T: AlefContract>(descriptor: &AlefComponentV1) -> Result<&T, ComponentError> {
    if descriptor.contract_hash != T::CONTRACT_HASH
        || descriptor.contract_size < core::mem::size_of::<T>()
        || descriptor.contract.is_null()
        || !(descriptor.contract as usize).is_multiple_of(core::mem::align_of::<T>())
    {
        return Err(ComponentError::InvalidContractTable);
    }
    Ok(unsafe { &*descriptor.contract.cast::<T>() })
}

fn process_pins() -> &'static Mutex<Vec<Arc<Library>>> {
    static PINS: OnceLock<Mutex<Vec<Arc<Library>>>> = OnceLock::new();
    PINS.get_or_init(|| Mutex::new(Vec::new()))
}

fn pin_for_process(library: Arc<Library>) {
    process_pins()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(library);
}

pub(crate) fn validate_manifest(
    manifest: &ComponentManifest,
    requirements: &ComponentRequirements,
) -> Result<(), ComponentError> {
    if manifest.schema_version != COMPONENT_MANIFEST_SCHEMA || manifest.abi_version != COMPONENT_ABI_VERSION {
        return Err(ComponentError::UnsupportedManifest);
    }
    if manifest.identity.component != requirements.component_id {
        return Err(ComponentError::IdentityMismatch {
            expected: requirements.component_id.clone(),
            actual: manifest.identity.component.clone(),
        });
    }
    if decode_hash(&manifest.identity.contract_hash)? != requirements.contract_hash {
        return Err(ComponentError::ContractHashMismatch);
    }
    if let Some(expected) = requirements.feature_set_hash
        && decode_hash(&manifest.identity.feature_hash)? != expected
    {
        return Err(ComponentError::FeatureSetHashMismatch);
    }
    Ok(())
}

unsafe fn validate_descriptor(
    descriptor: &AlefComponentV1,
    requirements: &ComponentRequirements,
) -> Result<(String, String), ComponentError> {
    if descriptor.struct_size < core::mem::size_of::<AlefComponentV1>() {
        return Err(ComponentError::InvalidDescriptor("descriptor is smaller than ABI v1"));
    }
    if descriptor.abi_major != ABI_MAJOR_V1 || descriptor.abi_minor > ABI_MINOR_V1 {
        return Err(ComponentError::IncompatibleAbi {
            expected_major: ABI_MAJOR_V1,
            expected_minor: ABI_MINOR_V1,
            actual_major: descriptor.abi_major,
            actual_minor: descriptor.abi_minor,
        });
    }
    let component_id = unsafe { copy_abi_str(descriptor.component_id)? };
    let component_version = unsafe { copy_abi_str(descriptor.component_version)? };
    if component_id != requirements.component_id {
        return Err(ComponentError::IdentityMismatch {
            expected: requirements.component_id.clone(),
            actual: component_id,
        });
    }
    if descriptor.contract_hash != requirements.contract_hash {
        return Err(ComponentError::ContractHashMismatch);
    }
    if requirements
        .feature_set_hash
        .is_some_and(|expected| descriptor.feature_set_hash != expected)
    {
        return Err(ComponentError::FeatureSetHashMismatch);
    }
    if descriptor.contract_size > 0 && descriptor.contract.is_null() {
        return Err(ComponentError::InvalidDescriptor(
            "non-empty contract has a null pointer",
        ));
    }
    Ok((component_id, component_version))
}

unsafe fn copy_abi_str(value: alef_component_abi::AlefStr) -> Result<String, ComponentError> {
    if value.len > MAX_DESCRIPTOR_STRING {
        return Err(ComponentError::StringTooLong);
    }
    if value.len == 0 {
        return Ok(String::new());
    }
    if value.ptr.is_null() {
        return Err(ComponentError::InvalidDescriptor("non-empty string has a null pointer"));
    }
    let bytes = unsafe { core::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) };
    let text = core::str::from_utf8(bytes).map_err(|_| ComponentError::InvalidUtf8)?;
    Ok(text.to_owned())
}

pub(crate) fn decode_hash(value: &str) -> Result<[u8; 32], ComponentError> {
    let bytes = hex::decode(value).map_err(|_| ComponentError::InvalidDigest(value.to_owned()))?;
    bytes
        .try_into()
        .map_err(|_| ComponentError::InvalidDigest(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_component_abi::{AlefComponentV1, AlefStr};
    use std::ffi::c_char;

    static ID: &[u8] = b"demo";
    static VERSION: &[u8] = b"1.2.3";

    #[repr(C)]
    struct DemoContract {
        answer: u32,
    }

    unsafe impl AlefContract for DemoContract {
        const CONTRACT_HASH: [u8; 32] = [7; 32];
    }

    static CONTRACT: DemoContract = DemoContract { answer: 42 };

    fn descriptor(contract_hash: [u8; 32]) -> AlefComponentV1 {
        AlefComponentV1 {
            struct_size: core::mem::size_of::<AlefComponentV1>(),
            abi_major: ABI_MAJOR_V1,
            abi_minor: ABI_MINOR_V1,
            component_id: AlefStr {
                ptr: ID.as_ptr().cast::<c_char>(),
                len: ID.len(),
            },
            component_version: AlefStr {
                ptr: VERSION.as_ptr().cast::<c_char>(),
                len: VERSION.len(),
            },
            contract_hash,
            feature_set_hash: [8; 32],
            contract: core::ptr::dangling(),
            contract_size: 1,
            create: None,
            destroy: None,
        }
    }

    fn requirements() -> ComponentRequirements {
        ComponentRequirements {
            component_id: "demo".into(),
            contract_hash: [7; 32],
            feature_set_hash: Some([8; 32]),
        }
    }

    #[test]
    fn accepts_matching_descriptor() {
        let descriptor = descriptor([7; 32]);
        let validated = unsafe { validate_descriptor(&descriptor, &requirements()) }.unwrap();
        assert_eq!(validated, ("demo".into(), "1.2.3".into()));
    }

    #[test]
    fn rejects_wrong_contract_hash() {
        let descriptor = descriptor([9; 32]);
        assert!(matches!(
            unsafe { validate_descriptor(&descriptor, &requirements()) },
            Err(ComponentError::ContractHashMismatch)
        ));
    }

    #[test]
    fn rejects_short_descriptor_before_optional_fields_are_read() {
        let mut descriptor = descriptor([7; 32]);
        descriptor.struct_size = 8;
        assert!(matches!(
            unsafe { validate_descriptor(&descriptor, &requirements()) },
            Err(ComponentError::InvalidDescriptor(_))
        ));
    }

    #[test]
    fn manifest_validation_checks_feature_identity() {
        let manifest = ComponentManifest {
            schema_version: 1,
            abi_version: 1,
            identity: crate::ComponentIdentity {
                crate_name: "demo-core".into(),
                component: "demo".into(),
                version: "1.2.3".into(),
                target: "test-target".into(),
                feature_hash: hex::encode([9; 32]),
                contract_hash: hex::encode([7; 32]),
            },
            contract: "engine".into(),
            contract_version: 1,
            implementation: "demo::Engine".into(),
            features: Vec::new(),
            default_features: false,
            library: crate::ComponentLibrary {
                file: "libdemo.so".into(),
                sha256: hex::encode([0; 32]),
                size: 0,
            },
        };
        assert!(matches!(
            validate_manifest(&manifest, &requirements()),
            Err(ComponentError::FeatureSetHashMismatch)
        ));
    }

    #[test]
    fn typed_contract_table_checks_hash_size_and_alignment() {
        let mut descriptor = descriptor(DemoContract::CONTRACT_HASH);
        descriptor.contract = (&raw const CONTRACT).cast();
        descriptor.contract_size = core::mem::size_of::<DemoContract>();
        assert_eq!(
            contract_table_from_descriptor::<DemoContract>(&descriptor)
                .unwrap()
                .answer,
            42
        );

        descriptor.contract_hash = [9; 32];
        assert!(matches!(
            contract_table_from_descriptor::<DemoContract>(&descriptor),
            Err(ComponentError::InvalidContractTable)
        ));
    }
}
