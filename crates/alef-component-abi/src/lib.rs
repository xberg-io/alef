//! Stable C ABI primitives shared by Alef component hosts and producers.

#![no_std]

use core::ffi::{c_char, c_void};

pub const ABI_MAJOR_V1: u32 = 1;
pub const ABI_MINOR_V1: u32 = 0;
pub const COMPONENT_ENTRYPOINT_V1: &[u8] = b"alef_component_entry_v1\0";

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlefStatus(pub i32);

impl AlefStatus {
    pub const OK: Self = Self(0);
    pub const INVALID_ARGUMENT: Self = Self(1);
    pub const INCOMPATIBLE_ABI: Self = Self(2);
    pub const INTERNAL_ERROR: Self = Self(3);
    pub const PENDING: Self = Self(4);
    pub const CANCELLED: Self = Self(5);

    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 == Self::OK.0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefStr {
    pub ptr: *const c_char,
    pub len: usize,
}

impl AlefStr {
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl AlefSlice {
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };
}

pub type AlefBufferFree = unsafe extern "C" fn(context: *mut c_void, ptr: *mut u8, len: usize, capacity: usize);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefOwnedBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
    pub context: *mut c_void,
    pub free: Option<AlefBufferFree>,
}

impl AlefOwnedBuffer {
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null_mut(),
        len: 0,
        capacity: 0,
        context: core::ptr::null_mut(),
        free: None,
    };
}

pub type AlefLogCallback = unsafe extern "C" fn(context: *mut c_void, level: u32, message: AlefStr);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefHostApiV1 {
    pub struct_size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
    pub context: *mut c_void,
    pub log: Option<AlefLogCallback>,
}

pub type AlefTaskCallback =
    unsafe extern "C" fn(user_data: *mut c_void, status: AlefStatus, result: AlefOwnedBuffer, error: AlefOwnedBuffer);
pub type AlefTaskStart =
    unsafe extern "C" fn(context: *mut c_void, callback: AlefTaskCallback, user_data: *mut c_void) -> AlefStatus;
pub type AlefTaskCancel = unsafe extern "C" fn(context: *mut c_void) -> AlefStatus;
pub type AlefTaskDrop = unsafe extern "C" fn(context: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefTaskV1 {
    pub struct_size: usize,
    pub context: *mut c_void,
    pub start: Option<AlefTaskStart>,
    pub cancel: Option<AlefTaskCancel>,
    pub drop: Option<AlefTaskDrop>,
}

pub type AlefComponentCreate = unsafe extern "C" fn(
    host: *const AlefHostApiV1,
    out_instance: *mut *mut c_void,
    out_error: *mut AlefOwnedBuffer,
) -> AlefStatus;
pub type AlefComponentDestroy = unsafe extern "C" fn(instance: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AlefComponentV1 {
    pub struct_size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
    pub component_id: AlefStr,
    pub component_version: AlefStr,
    pub contract_hash: [u8; 32],
    pub feature_set_hash: [u8; 32],
    pub contract: *const c_void,
    pub contract_size: usize,
    pub create: Option<AlefComponentCreate>,
    pub destroy: Option<AlefComponentDestroy>,
}

pub type AlefComponentEntryV1 = unsafe extern "C" fn(
    requested_abi_major: u32,
    host: *const AlefHostApiV1,
    out_component: *mut *const AlefComponentV1,
) -> AlefStatus;

/// Marker implemented by generated `repr(C)` contract tables.
///
/// # Safety
///
/// Implementors must be plain C-compatible function tables whose layout is
/// stable for `CONTRACT_HASH`. They must not contain Rust references or types
/// with an unspecified representation.
pub unsafe trait AlefContract {
    const CONTRACT_HASH: [u8; 32];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_accepts_unknown_values_without_enum_ub() {
        let vendor_status = AlefStatus(44_001);
        assert!(!vendor_status.is_ok());
        assert_eq!(vendor_status.0, 44_001);
    }

    #[test]
    fn abi_structs_have_c_compatible_alignment() {
        assert_eq!(core::mem::align_of::<AlefStatus>(), core::mem::align_of::<i32>());
        assert_eq!(
            core::mem::align_of::<AlefComponentV1>(),
            core::mem::align_of::<*const c_void>()
        );
        assert_eq!(
            core::mem::size_of::<AlefComponentV1>() % core::mem::align_of::<AlefComponentV1>(),
            0
        );
        assert_eq!(
            core::mem::size_of::<AlefTaskV1>() % core::mem::align_of::<AlefTaskV1>(),
            0
        );
    }
}
