use alef_component_abi::{AlefContract, AlefHostApiV1, AlefOwnedBuffer, AlefSlice, AlefStatus};
use alef_component_runtime::{ComponentLock, ComponentManager};
use std::ffi::c_void;

#[repr(C)]
struct BytesContract {
    struct_size: usize,
    transform:
        Option<unsafe extern "C" fn(*mut c_void, AlefSlice, *mut AlefOwnedBuffer, *mut AlefOwnedBuffer) -> AlefStatus>,
}

// SAFETY: the canonical conformance contract is a C-layout table with one bytes method. ~keep
unsafe impl AlefContract for BytesContract {
    const CONTRACT_HASH: [u8; 32] = CONTRACT_HASH;
}

const CONTRACT_HASH: [u8; 32] = include!("contract_hash.rs");

unsafe fn release(buffer: AlefOwnedBuffer) -> Vec<u8> {
    assert!(buffer.len <= buffer.capacity, "buffer length exceeds capacity");
    if buffer.len == 0 {
        if let Some(free) = buffer.free {
            unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
        }
        return Vec::new();
    }
    assert!(!buffer.ptr.is_null(), "nonempty buffer has null pointer");
    let free = buffer.free.expect("nonempty owned buffer must provide a free callback");
    let bytes = unsafe { std::slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec();
    unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
    bytes
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(args.len(), 5, "expected lock, cache, target, component, contract");
    let lock: ComponentLock = serde_json::from_slice(&std::fs::read(&args[0]).unwrap()).unwrap();
    let manager = ComponentManager::from_lock(lock, &args[1], &args[2], AlefHostApiV1::EMPTY).unwrap();
    let entry = format!("alef_component_entry_v1_{}\0", args[4]);
    let loaded = manager
        .ensure_contract(&args[3], &args[4], entry.as_bytes())
        .expect("load conformance producer");
    assert_eq!(loaded.component_id(), args[3]);
    assert_eq!(loaded.descriptor().abi_major, 1);
    assert_eq!(loaded.descriptor().contract_hash, CONTRACT_HASH);
    let instance = loaded
        .instantiate::<BytesContract>()
        .expect("create conformance instance");
    let transform = instance.table().transform.expect("bytes contract method is required");
    let mut calls = 0;
    for (input, status, expected) in [
        (b"ab\0cd".as_slice(), AlefStatus::OK, b"dc\0ba".as_slice()),
        (b"error", AlefStatus::INTERNAL_ERROR, b"conformance error"),
        (b"panic", AlefStatus::INTERNAL_ERROR, b"conformance panic"),
        (b"after", AlefStatus::OK, b"retfa"),
        (b"", AlefStatus::OK, b""),
    ] {
        let mut result = AlefOwnedBuffer::EMPTY;
        let mut error = AlefOwnedBuffer::EMPTY;
        let actual = unsafe {
            transform(
                instance.handle(),
                AlefSlice {
                    ptr: input.as_ptr(),
                    len: input.len(),
                },
                &mut result,
                &mut error,
            )
        };
        assert_eq!(actual, status, "status for {input:?}");
        let result = unsafe { release(result) };
        let error = unsafe { release(error) };
        if status.is_ok() {
            assert_eq!(result, expected);
            assert!(error.is_empty());
        } else {
            assert!(result.is_empty());
            assert!(
                String::from_utf8(error)
                    .unwrap()
                    .contains(std::str::from_utf8(expected).unwrap())
            );
        }
        calls += 1;
    }
    assert_eq!(calls, 5, "all normal/error/panic/recovery/empty calls must fire");
    drop(instance);
    drop(
        loaded
            .instantiate::<BytesContract>()
            .expect("instance can be recreated after destroy"),
    );
    println!("COMPONENT_CONFORMANCE_OK load describe call error panic buffer-free calls={calls}");
}
