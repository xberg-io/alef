//! Pins the C-compatible layout of every `#[repr(C)]` struct this crate exports against the
//! field order and widths the hand-typed C header in `alef`'s
//! `src/codegen/component.rs::c_header` declares for them.
//!
//! `alef-component-abi` cannot depend on the `alef` crate (`c_header` lives there, and `alef`
//! depends on this crate, not the other way around), so this test cannot literally render
//! that header. Instead it hardcodes the field sizes and order the header text currently
//! spells out and checks this crate's structs via `size_of`/`align_of`/`offset_of!` against
//! them, so a field reorder in either this crate's structs or that header's hand-typed text
//! shows up as a size or offset mismatch here. A complementary alef-side test parses the
//! rendered header text itself and checks it against the same field order.
//!
//! `struct_size` is additionally pinned at offset 0 in every versioned struct that carries
//! one: `alef_component_runtime::loader` reads it through a raw pointer before a full
//! reference to the struct is ever formed, which is only sound while it is the first field.

use alef_component_abi::{AlefComponentV1, AlefHostApiV1, AlefOwnedBuffer, AlefSlice, AlefStr, AlefTaskV1};
use core::mem::{align_of, offset_of, size_of};

/// Width of a bare pointer or C function pointer on this target -- what the header's `size_t`,
/// bare `*`, and function-pointer fields all resolve to when compiled for the same target as
/// this crate.
const PTR: usize = size_of::<*const ()>();

#[test]
fn alef_str_matches_c_header_layout() {
    // `typedef struct { const char *ptr; size_t len; } AlefComponentStr;`
    assert_eq!(offset_of!(AlefStr, ptr), 0);
    assert_eq!(offset_of!(AlefStr, len), PTR);
    assert_eq!(size_of::<AlefStr>(), 2 * PTR);
    assert_eq!(align_of::<AlefStr>(), PTR);
}

#[test]
fn alef_slice_matches_c_header_layout() {
    // `typedef struct { const uint8_t *ptr; size_t len; } AlefComponentSlice;`
    assert_eq!(offset_of!(AlefSlice, ptr), 0);
    assert_eq!(offset_of!(AlefSlice, len), PTR);
    assert_eq!(size_of::<AlefSlice>(), 2 * PTR);
    assert_eq!(align_of::<AlefSlice>(), PTR);
}

#[test]
fn alef_owned_buffer_matches_c_header_layout() {
    // `typedef struct { uint8_t *ptr; size_t len; size_t capacity; void *context;
    //   AlefComponentBufferFree free; } AlefComponentOwnedBuffer;`
    assert_eq!(offset_of!(AlefOwnedBuffer, ptr), 0);
    assert_eq!(offset_of!(AlefOwnedBuffer, len), PTR);
    assert_eq!(offset_of!(AlefOwnedBuffer, capacity), 2 * PTR);
    assert_eq!(offset_of!(AlefOwnedBuffer, context), 3 * PTR);
    assert_eq!(offset_of!(AlefOwnedBuffer, free), 4 * PTR);
    assert_eq!(size_of::<AlefOwnedBuffer>(), 5 * PTR);
    assert_eq!(align_of::<AlefOwnedBuffer>(), PTR);
}

#[test]
fn alef_host_api_v1_matches_c_header_layout_and_leads_with_struct_size() {
    // `typedef struct { size_t struct_size; uint32_t abi_major; uint32_t abi_minor;
    //   void *context; void (*log)(void *, uint32_t, AlefComponentStr); } AlefComponentHostApiV1;`
    assert_eq!(
        offset_of!(AlefHostApiV1, struct_size),
        0,
        "struct_size must stay the first field: callers validate it through a raw read \
         before forming a full reference to this struct"
    );
    assert_eq!(offset_of!(AlefHostApiV1, abi_major), PTR);
    assert_eq!(offset_of!(AlefHostApiV1, abi_minor), PTR + size_of::<u32>());
    assert_eq!(offset_of!(AlefHostApiV1, context), 2 * PTR);
    assert_eq!(offset_of!(AlefHostApiV1, log), 3 * PTR);
    assert_eq!(size_of::<AlefHostApiV1>(), 4 * PTR);
    assert_eq!(align_of::<AlefHostApiV1>(), PTR);
}

#[test]
fn alef_task_v1_matches_c_header_layout_and_leads_with_struct_size() {
    // `typedef struct AlefComponentTaskV1 { size_t struct_size; void *context;
    //   AlefComponentTaskStart start; AlefComponentStatus (*cancel)(void *);
    //   void (*drop)(void *); } AlefComponentTaskV1;`
    assert_eq!(
        offset_of!(AlefTaskV1, struct_size),
        0,
        "struct_size must stay the first field: callers validate it through a raw read \
         before forming a full reference to this struct"
    );
    assert_eq!(offset_of!(AlefTaskV1, context), PTR);
    assert_eq!(offset_of!(AlefTaskV1, start), 2 * PTR);
    assert_eq!(offset_of!(AlefTaskV1, cancel), 3 * PTR);
    assert_eq!(offset_of!(AlefTaskV1, drop), 4 * PTR);
    assert_eq!(size_of::<AlefTaskV1>(), 5 * PTR);
    assert_eq!(align_of::<AlefTaskV1>(), PTR);
}

#[test]
fn alef_component_v1_matches_c_header_layout_and_leads_with_struct_size() {
    // `typedef struct { size_t struct_size; uint32_t abi_major; uint32_t abi_minor;
    //   AlefComponentStr component_id; AlefComponentStr component_version;
    //   uint8_t contract_hash[32]; uint8_t feature_set_hash[32]; const void *contract;
    //   size_t contract_size;
    //   AlefComponentStatus (*create)(const AlefComponentHostApiV1 *, void **, AlefComponentOwnedBuffer *);
    //   void (*destroy)(void *); } AlefComponentV1;`
    assert_eq!(
        offset_of!(AlefComponentV1, struct_size),
        0,
        "struct_size must stay the first field: alef_component_runtime::loader validates it \
         through a raw read before forming a full `&AlefComponentV1` reference"
    );
    assert_eq!(offset_of!(AlefComponentV1, abi_major), PTR);
    assert_eq!(offset_of!(AlefComponentV1, abi_minor), PTR + size_of::<u32>());
    assert_eq!(offset_of!(AlefComponentV1, component_id), 2 * PTR);

    let after_component_id = 2 * PTR + size_of::<AlefStr>();
    assert_eq!(offset_of!(AlefComponentV1, component_version), after_component_id);

    let after_strings = after_component_id + size_of::<AlefStr>();
    assert_eq!(offset_of!(AlefComponentV1, contract_hash), after_strings);
    assert_eq!(offset_of!(AlefComponentV1, feature_set_hash), after_strings + 32);

    let after_hashes = after_strings + 64;
    assert_eq!(offset_of!(AlefComponentV1, contract), after_hashes);
    assert_eq!(offset_of!(AlefComponentV1, contract_size), after_hashes + PTR);
    assert_eq!(offset_of!(AlefComponentV1, create), after_hashes + 2 * PTR);
    assert_eq!(offset_of!(AlefComponentV1, destroy), after_hashes + 3 * PTR);
    assert_eq!(size_of::<AlefComponentV1>(), after_hashes + 4 * PTR);
    assert_eq!(align_of::<AlefComponentV1>(), PTR);
}
