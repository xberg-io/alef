use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// The emitted `lib.rs`'s crate-level `#![allow(...)]` list was audited against a real
/// `cargo clippy --all-targets -- -D warnings` run over an emitted tree (see
/// `tests/generated_output_downstream_gate.rs`), removing entries that never fired. This
/// pins the ones removed so they cannot silently return -- if one of them becomes
/// load-bearing again, the fix belongs at the emitter or per-item allow, not here. ~keep
#[test]
fn crate_level_allow_list_does_not_carry_dead_entries() {
    let api = ApiSurface {
        crate_name: "sample_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![FunctionDef {
            name: "run".to_string(),
            rust_path: "sample_lib::run".to_string(),
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let header_end = lib.content.find("use std::ffi").unwrap_or(lib.content.len());
    let header = &lib.content[..header_end];

    assert!(
        !header.contains("missing_docs"),
        "missing_docs is allow-by-default under rustc and is never escalated by -D warnings \
         alone, so the crate-level allow was a no-op:\n{header}"
    );
    assert!(
        !header.contains("clippy::too_many_arguments"),
        "too_many_arguments already gets a per-item #[allow] at every site that can exceed \
         the threshold (free functions, len companions, method wrappers, constructors, field \
         accessors), so the crate-level copy was redundant:\n{header}"
    );
    assert!(
        !header.contains("clippy::useless_conversion"),
        "useless_conversion's only source was the Vec<u8>::from(..) polymorphic bytes \
         conversion in bytes_result_match.jinja, which now copies through a `&[u8]` slice \
         at every one of its four sites -- a real conversion for every input shape, so no \
         allow is needed anywhere:\n{header}"
    );
    // `unnecessary_cast` fires only when a cast's source expression already has the exact
    // destination type. Every `as i32`/`as u32`/`as usize` this backend emits casts a `bool`
    // or a genuine `Named` enum field (see `ffi_visitor_context_enum_init.jinja`, reached only
    // when `context_c_type` (`gen_visitor/context.rs`) resolves the field to a real IR enum --
    // never when the field is already `TypeRef::Primitive(I32)`, which routes through the
    // cast-free passthrough template instead) or an integer of a different width
    // (`handle_registry.rs.jinja`'s `u64 -> usize`/`u64 -> u32`). None of those is ever the
    // same type as the cast target, so clippy can never see this as redundant. Verified against
    // a real `cargo clippy --all-targets -- -D warnings` run over the non-visitor emitted tree
    // (`tests/generated_output_downstream_gate.rs`'s fixture) and, for the enum-context site
    // specifically, over a real `alef generate` run with a configured trait bridge --
    // `gen_bindings::tests::visitor::test_visitor_callbacks_emit_enum_node_type_as_i32` pins
    // that this backend still reaches and emits that exact cast. ~keep
    assert!(
        !header.contains("clippy::unnecessary_cast"),
        "every cast this backend emits converts a bool or a Named enum to a different \
         primitive type, which clippy's same-type check can never flag as redundant:\n{header}"
    );
    // `dropping_references` fires only when `drop(...)`'s argument is itself a reference
    // (`&T`/`&mut T`); dropping a reference is a no-op the lint exists to catch. Every explicit
    // `drop(...)` this backend emits drops an owned value instead: `free_bytes.jinja`'s
    // `Box::<[u8]>::from_raw(..)`, `free_string.jinja`'s `CString::from_raw(..)`,
    // `handle_registry.rs.jinja`'s `self.take::<T>(handle)?` and `guard` (an owned
    // `MutexGuard`, not a borrow of one), and `orchestration.rs`'s `std::mem::drop(obj)` for a
    // method literally named `drop`, where `obj` comes from `null_check_self_owned.jinja`'s
    // `take_handle::<T>(this)` -- an owned `T`, never a reference binding. No template or
    // generator path in this backend ever passes a reference to `drop(...)`, so the entry never
    // had anything to allow. Confirmed with a real `cargo clippy --all-targets -- -D warnings`
    // run over the gate fixture (`tests/generated_output_downstream_gate.rs`) with the entry
    // removed: no new warning appeared. Removed and pinned here per this test's own contract. ~keep
    assert!(
        !header.contains("dropping_references"),
        "every drop(...) this backend emits drops an owned value, never a reference, so the \
         crate-level allow was a no-op:\n{header}"
    );
}

/// The paired positive: the narrow per-item allow this audit added actually reaches the
/// generated bytes-conversion call site.
#[test]
fn bytes_result_conversion_carries_its_own_narrow_useless_conversion_allow() {
    let api = ApiSurface {
        crate_name: "sample_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "sample_lib::render".to_string(),
            return_type: TypeRef::Bytes,
            error_type: Some("String".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    // The bytes conversion copies through `&val[..]` so it also accepts a borrowing accessor
    // (`fn as_bytes(&self) -> &Bytes`). That is a real conversion for every input shape, so the
    // narrow `useless_conversion` allow it used to need is gone -- assert it is not reintroduced
    // alongside the crate-level one this test exists to keep deleted. ~keep
    assert!(
        lib.content
            .contains("let buffer = Vec::<u8>::from(&val[..]).into_boxed_slice();"),
        "the bytes-conversion call site must copy through a slice:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("clippy::useless_conversion"),
        "the slice copy is never a useless conversion; the allow must not come back:\n{}",
        lib.content
    );
}

/// The infallible, non-optional bytes return is the fourth site in `bytes_result_match.jinja`
/// and the one the other three do not cover: it binds `result` rather than `val`, so a fix
/// applied only to the `match` arms leaves it converting an owned-only `Vec::<u8>::from(result)`.
/// A borrowing accessor (`fn as_bytes(&self) -> &Bytes`) then fails to compile in the generated
/// crate -- exactly the break the arm fix was made to prevent. ~keep
#[test]
fn infallible_bytes_return_also_copies_through_a_slice() {
    let api = ApiSurface {
        crate_name: "sample_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "sample_lib::render".to_string(),
            return_type: TypeRef::Bytes,
            error_type: None,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("let buffer = Vec::<u8>::from(&result[..]).into_boxed_slice();"),
        "the infallible bytes call site must copy through a slice:\n{}",
        lib.content
    );
    // Matched against the whole call site, not the bare `Vec::<u8>::from(result)` prefix: the
    // `~keep` comment above the line names the old form to explain why it went, and a substring
    // check would fire on the explanation rather than on a regression. ~keep
    assert!(
        !lib.content.contains("Vec::<u8>::from(result).into_boxed_slice()"),
        "the owned-only conversion rejects a borrowing `&Bytes` accessor:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("clippy::useless_conversion"),
        "the slice copy is never a useless conversion; the allow must not come back:\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("infallible bytes wrapper must parse as valid Rust");
}
