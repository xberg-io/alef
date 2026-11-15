use super::*;
use alef_component_abi::{AlefComponentV1, AlefStr};
use std::ffi::c_char;

static ID: &[u8] = b"demo";
static VERSION: &[u8] = b"1.2.3";

#[repr(C)]
struct DemoContract {
    answer: u32,
}

// SAFETY: `DemoContract` is a `repr(C)` table and these tests pair it only with its own hash.
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
        contract_name: "engine".into(),
        contract_hash: [7; 32],
        feature_set_hash: Some([8; 32]),
    }
}

#[test]
fn accepts_matching_descriptor() {
    let descriptor = descriptor([7; 32]);
    // SAFETY: the descriptor's strings point at the static `ID` and `VERSION`.
    let validated = unsafe { validate_descriptor(&descriptor, &requirements()) }.unwrap();
    assert_eq!(validated, ("demo".into(), "1.2.3".into()));
}

#[test]
fn rejects_wrong_contract_hash() {
    let descriptor = descriptor([9; 32]);
    // SAFETY: the descriptor's strings point at the static `ID` and `VERSION`.
    let result = unsafe { validate_descriptor(&descriptor, &requirements()) };
    assert!(matches!(result, Err(ComponentError::ContractHashMismatch)));
}

#[test]
fn rejects_short_descriptor_before_optional_fields_are_read() {
    let mut descriptor = descriptor([7; 32]);
    descriptor.struct_size = 8;
    // SAFETY: the descriptor's strings point at the static `ID` and `VERSION`.
    let result = unsafe { validate_descriptor(&descriptor, &requirements()) };
    assert!(matches!(result, Err(ComponentError::InvalidDescriptor(_))));
}

/// Regression for forming a `&AlefComponentV1` over memory that only actually holds a
/// `struct_size` field. The backing allocation below is deliberately sized for nothing
/// more than that one `usize`, so `reject_undersized_descriptor`'s raw pointer read must
/// be the only access that happens -- ever forming a full `&AlefComponentV1` reference
/// over it would read past the allocation.
#[test]
fn rejects_a_descriptor_backed_by_memory_too_small_to_hold_the_full_struct() {
    let layout =
        std::alloc::Layout::from_size_align(core::mem::size_of::<usize>(), core::mem::align_of::<AlefComponentV1>())
            .unwrap();
    // SAFETY: `layout` has non-zero size.
    let raw = unsafe { std::alloc::alloc(layout) };
    assert!(!raw.is_null(), "test allocation failed");
    // SAFETY: `raw` is valid for `size_of::<usize>()` bytes per `layout` and is
    // sufficiently aligned for a `usize` write.
    unsafe { raw.cast::<usize>().write_unaligned(8) };
    let descriptor = NonNull::new(raw).unwrap().cast::<AlefComponentV1>();

    // SAFETY: `descriptor` points to `size_of::<usize>()` valid bytes, matching this
    // function's caller contract; no wider reference is formed here.
    let result = unsafe { reject_undersized_descriptor(descriptor) };

    // SAFETY: `raw`/`layout` match the earlier allocation exactly.
    unsafe { std::alloc::dealloc(raw, layout) };
    assert!(matches!(result, Err(ComponentError::InvalidDescriptor(_))));
}

#[test]
fn manifest_validation_checks_feature_identity() {
    let manifest = ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: 1,
        identity: crate::ComponentIdentity {
            crate_name: "demo-core".into(),
            component: "demo".into(),
            version: "1.2.3".into(),
            target: "test-target".into(),
            feature_hash: hex::encode([9; 32]),
            contract_hash: hex::encode([7; 32]),
        },
        provides: vec![crate::ComponentProvidedContract {
            contract: "engine".into(),
            interface_version: 1,
            contract_hash: hex::encode([7; 32]),
            implementation: "demo::Engine".into(),
        }],
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

#[test]
fn descriptor_strings_up_to_the_limit_are_copied_and_longer_ones_refused() {
    let at_limit = vec![b'a'; MAX_DESCRIPTOR_STRING];
    let value = AlefStr {
        ptr: at_limit.as_ptr().cast::<c_char>(),
        len: at_limit.len(),
    };
    // SAFETY: `value` points at `at_limit`, which is live and `len` bytes long.
    let copied = unsafe { copy_abi_str(value) };
    assert_eq!(copied.unwrap().len(), MAX_DESCRIPTOR_STRING);

    let over = AlefStr {
        ptr: at_limit.as_ptr().cast::<c_char>(),
        len: MAX_DESCRIPTOR_STRING + 1,
    };
    // SAFETY: the length is rejected before `ptr` is read, so the one byte past `at_limit` is
    // never touched.
    let result = unsafe { copy_abi_str(over) };
    assert!(matches!(result, Err(ComponentError::StringTooLong)));
}

#[test]
fn descriptor_strings_must_be_utf8_and_non_null_when_non_empty() {
    let invalid = [0xff_u8, 0xfe];
    let value = AlefStr {
        ptr: invalid.as_ptr().cast::<c_char>(),
        len: invalid.len(),
    };
    // SAFETY: `value` points at `invalid`, which is live and `len` bytes long.
    let result = unsafe { copy_abi_str(value) };
    assert!(matches!(result, Err(ComponentError::InvalidUtf8)));
    let null = AlefStr {
        ptr: core::ptr::null(),
        len: 1,
    };
    // SAFETY: a null pointer with a non-zero length is rejected before it is read.
    let result = unsafe { copy_abi_str(null) };
    assert!(matches!(result, Err(ComponentError::InvalidDescriptor(_))));
}

/// ADR-0015: within ABI major 1, a host loads components built for its own or an older minor and
/// refuses a newer one, whose descriptor may rely on host-table fields this host does not have.
#[test]
fn a_newer_abi_minor_or_another_major_is_refused() {
    let mut newer = descriptor([7; 32]);
    newer.abi_minor = ABI_MINOR_V1 + 1;
    // SAFETY: the descriptor's strings point at the static `ID` and `VERSION`.
    let result = unsafe { validate_descriptor(&newer, &requirements()) };
    assert!(matches!(
        result,
        Err(ComponentError::IncompatibleAbi { actual_minor, .. }) if actual_minor == ABI_MINOR_V1 + 1
    ));
    let mut other_major = descriptor([7; 32]);
    other_major.abi_major = ABI_MAJOR_V1 + 1;
    // SAFETY: the descriptor's strings point at the static `ID` and `VERSION`.
    let result = unsafe { validate_descriptor(&other_major, &requirements()) };
    assert!(matches!(result, Err(ComponentError::IncompatibleAbi { .. })));
}

/// Regression: every load pushed another `Arc<Library>` onto the process pins, so a host that
/// re-ensured contracts grew the list without bound. One pin per verified file now.
#[cfg(unix)]
#[test]
fn a_library_file_is_pinned_once() {
    let dir = tempfile::tempdir().unwrap();
    let identity = |name: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, name).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let metadata = file.metadata().unwrap();
        VerifiedFile::new(path, file, &metadata).identity()
    };
    let this = || Arc::new(Library::from(libloading::os::unix::Library::this()));
    let (first, second) = (identity("a.so"), identity("b.so"));

    let mut pins = HashMap::new();
    assert!(pin_into(&mut pins, first.clone(), this()));
    assert!(!pin_into(&mut pins, first, this()));
    assert!(pin_into(&mut pins, second, this()));
    assert_eq!(pins.len(), 2);
}
