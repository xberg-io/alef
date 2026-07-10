//! Integration tests for the C-ABI capsule (Language-passthrough) feature.
//!
//! Verifies that when `[crates.ffi.capsule_types]` lists a type, the generated lib.rs:
//!   - returns the host runtime's raw grammar pointer (`*const tree_sitter::ffi::TSLanguage`)
//!     from the exported C function instead of boxing an opaque `*mut Language` handle,
//!   - calls `value.into_raw()` (no `Box::into_raw`),
//!   - suppresses the opaque `_free` / `_to_json` symbols for the capsule type,
//!     and that the generated cbindgen.toml forward-declares the unprefixed pointee type.

use super::super::FfiBackend;
use super::common::resolved_one;
use crate::core::backend::Backend;
use crate::core::ir::*;

fn capsule_api() -> ApiSurface {
    ApiSurface {
        crate_name: "ts-pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "ts_pack::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "A tree-sitter grammar.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "get_language".to_string(),
            rust_path: "ts_pack::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: "Look up a grammar by name.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        ..Default::default()
    }
}

fn capsule_config() -> crate::core::config::ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "ts-pack"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tsp"

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
"#,
    )
}

#[test]
fn capsule_function_returns_raw_language_pointer() {
    let api = capsule_api();
    let config = capsule_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("-> *const tree_sitter::ffi::TSLanguage"),
        "capsule fn must return *const tree_sitter::ffi::TSLanguage. Got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("pub unsafe extern \"C\" fn tsp_get_language("),
        "expected exported tsp_get_language symbol"
    );
}

#[test]
fn capsule_function_calls_into_raw_not_box() {
    let api = capsule_api();
    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("result.into_raw() as *const tree_sitter::ffi::TSLanguage"),
        "capsule fn must convert via into_raw() cast. Got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("Box::into_raw(Box::new(result))"),
        "capsule fn must NOT box the value into an opaque handle"
    );
}

#[test]
fn capsule_type_suppresses_opaque_lifecycle_symbols() {
    let api = capsule_api();
    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("tsp_language_free"),
        "capsule type must not emit an opaque _free symbol"
    );
    assert!(
        !lib.content.contains("tsp_language_to_json"),
        "capsule type must not emit an opaque _to_json symbol"
    );
}

#[test]
fn cbindgen_forward_declares_unprefixed_pointee() {
    let api = capsule_api();
    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen.content.contains("typedef struct TSLanguage TSLanguage;"),
        "cbindgen.toml must forward-declare the unprefixed TSLanguage pointee. Got:\n{}",
        cbindgen.content
    );
    assert!(
        !cbindgen.content.contains("typedef struct TSPLanguage TSPLanguage;"),
        "capsule type must NOT be emitted as a prefixed opaque handle typedef"
    );
}
