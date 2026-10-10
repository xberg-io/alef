//! Companion to `crates/alef-component-abi/tests/layout_test.rs`: that test pins
//! `alef_component_abi`'s `#[repr(C)]` structs against a hardcoded copy of the field order
//! `ComponentContractIr::c_header` hand-types for them (the abi crate cannot depend on
//! `alef`, so it cannot render the header itself). This test closes the other side of that
//! gap by actually rendering `c_header()` and checking its hand-typed struct declarations
//! list fields in the same order as their `alef_component_abi` Rust counterparts
//! (`crates/alef-component-abi/src/lib.rs`) -- a cbindgen-free regression for the two
//! hand-written copies drifting apart.

use alef::codegen::component::ComponentContractIr;
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// Render `c_header()` for the smallest possible contract. The three versioned structs this
/// test checks are part of the header's fixed preamble, emitted unconditionally before any
/// contract-specific content, so a contract with no params, records, or async methods is
/// enough to exercise them.
fn rendered_header() -> String {
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Probe".into(),
            rust_path: "demo::Probe".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "ping".into(),
                return_type: TypeRef::Unit,
                receiver: Some(ReceiverKind::Ref),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    ComponentContractIr::from_trait(&api, "probe", "demo::Probe", 1)
        .expect("minimal probe contract must extract")
        .c_header()
}

/// The ordered field *names* declared inside the `typedef struct { ... } <type_name>;` (or
/// `typedef struct <type_name> { ... } <type_name>;`) block in `header`.
///
/// Deliberately naive text parsing over a fixed, hand-typed header -- exactly precise enough
/// to catch a field being reordered, renamed, or removed, without a cbindgen dependency.
fn struct_field_names(header: &str, type_name: &str) -> Vec<String> {
    let needle = format!("}} {type_name};");
    let close = header
        .find(&needle)
        .unwrap_or_else(|| panic!("struct `{type_name}` not found in header:\n{header}"));
    let open = header[..close]
        .rfind('{')
        .unwrap_or_else(|| panic!("no opening brace found for struct `{type_name}`"));
    header[open + 1..close]
        .split(';')
        .map(str::trim)
        .filter(|declaration| !declaration.is_empty())
        .map(field_name)
        .collect()
}

/// The declared name in one C field declaration, e.g. `context` from `void *context` and
/// `log` from `void (*log)(void *, uint32_t, AlefComponentStr)`.
fn field_name(declaration: &str) -> String {
    if let Some(start) = declaration.find("(*") {
        let rest = &declaration[start + 2..];
        let end = rest
            .find(')')
            .unwrap_or_else(|| panic!("function-pointer field `{declaration}` never closes its name group"));
        return rest[..end].to_string();
    }
    declaration
        .split('[') // drop a trailing array suffix, e.g. `contract_hash[32]`
        .next()
        .unwrap_or(declaration)
        .trim()
        .rsplit(|c: char| c.is_whitespace() || c == '*')
        .find(|token| !token.is_empty())
        .unwrap_or_else(|| panic!("could not find a field name in `{declaration}`"))
        .to_string()
}

#[test]
fn host_api_v1_header_fields_match_alef_component_abi_declaration_order() {
    assert_eq!(
        struct_field_names(&rendered_header(), "AlefComponentHostApiV1"),
        ["struct_size", "abi_major", "abi_minor", "context", "log"],
        "AlefComponentHostApiV1 in c_header() must list fields in the same order as \
         alef_component_abi::AlefHostApiV1"
    );
}

#[test]
fn task_v1_header_fields_match_alef_component_abi_declaration_order() {
    assert_eq!(
        struct_field_names(&rendered_header(), "AlefComponentTaskV1"),
        ["struct_size", "context", "start", "cancel", "drop"],
        "AlefComponentTaskV1 in c_header() must list fields in the same order as \
         alef_component_abi::AlefTaskV1"
    );
}

#[test]
fn component_v1_header_fields_match_alef_component_abi_declaration_order() {
    assert_eq!(
        struct_field_names(&rendered_header(), "AlefComponentV1"),
        [
            "struct_size",
            "abi_major",
            "abi_minor",
            "component_id",
            "component_version",
            "contract_hash",
            "feature_set_hash",
            "contract",
            "contract_size",
            "create",
            "destroy",
        ],
        "AlefComponentV1 in c_header() must list fields in the same order as \
         alef_component_abi::AlefComponentV1"
    );
}
