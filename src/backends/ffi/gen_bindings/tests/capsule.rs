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
            serde_container_default: false,
            serde_deserialization: Default::default(),
            serde_container_conversion: Default::default(),
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
            error_type: Some("SampleError".to_string()),
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
    assert!(
        lib.content.contains("return std::ptr::null();"),
        "capsule parameter failures must return a const null pointer"
    );
}

/// `val.into_raw()` must be called bare, with no `as *const {into_raw_type}` cast appended.
/// `into_raw_type` is documented as the pointee type `value.into_raw()` already returns (see
/// `FfiCapsuleTypeConfig::into_raw_type`), and tree-sitter's own
/// `Language::into_raw(self) -> *const ffi::TSLanguage` confirms it for this fixture -- so
/// `val.into_raw() as *const tree_sitter::ffi::TSLanguage` is a same-type cast that trips
/// `clippy::unnecessary_cast` under `-D warnings`. Regression coverage for the
/// tree-sitter-language-pack sighting; this test was red before the fix (it asserted the
/// cast WAS present).
#[test]
fn capsule_function_calls_into_raw_not_box() {
    let api = capsule_api();
    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("val.into_raw()"),
        "capsule fn must convert via into_raw(). Got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("val.into_raw() as"),
        "capsule fn must not append a redundant cast to into_raw()'s own type. Got:\n{}",
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
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        cbindgen.content.contains("typedef struct TSLanguage TSLanguage;"),
        "cbindgen.toml must forward-declare the unprefixed TSLanguage pointee. Got:\n{}",
        cbindgen.content
    );
    assert!(
        !cbindgen.content.contains("typedef struct TSPLanguage TSPLanguage;"),
        "capsule type must NOT be emitted as a prefixed opaque handle typedef"
    );
    assert!(
        lib.content.contains("*const tree_sitter::ffi::TSLanguage"),
        "the forward-declared TSLanguage typedef must have a real generated function that uses \
         it as its return type, got lib.rs:\n{}",
        lib.content
    );
}

/// Regression coverage for an orphaned capsule typedef: `[crates.ffi.capsule_types]` still lists
/// a type after the function that used to return it was removed, renamed, or excluded (a stale
/// config entry). Before this fix, `gen_cbindgen_toml`'s capsule forward-declaration block
/// unconditionally forward-declared `c_return_type` for every entry in `config.ffi.capsule_types`
/// -- reading only the static config, never the actual API surface -- so the header kept
/// declaring `typedef struct TSLanguage TSLanguage;` even though zero generated functions
/// referenced it. A C consumer got a type it could neither construct (no function returns it)
/// nor pass anywhere (no function takes it): "declared but unusable", not "declared and
/// under-implemented" -- so the fix removes the orphan declaration rather than inventing a
/// function nothing in the source ever asked for.
#[test]
fn capsule_c_return_type_typedef_omitted_when_no_function_returns_it() {
    let mut api = capsule_api();
    // No function/method returns `Language` (the capsule type) -- simulates a stale
    // `[crates.ffi.capsule_types.Language]` entry left behind after `get_language` was removed
    // or renamed.
    api.functions.clear();

    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !cbindgen.content.contains("TSLanguage"),
        "an unused capsule type's c_return_type must not be forward-declared -- no generated \
         function will ever reference it, got cbindgen.toml:\n{}",
        cbindgen.content
    );
    assert!(
        !lib.content.contains("TSLanguage") && !lib.content.contains("tree_sitter"),
        "sanity check: no function anywhere actually returns the capsule type in this fixture, \
         got lib.rs:\n{}",
        lib.content
    );
}

/// Same fixture, but with a second, still-used function alongside the stale capsule config --
/// the fix must be scoped to the specific unused capsule entry, not disable forward declarations
/// for capsule types wholesale.
#[test]
fn capsule_c_return_type_typedef_kept_for_still_used_entry_alongside_a_stale_one() {
    let mut api = capsule_api();
    api.functions.push(FunctionDef {
        name: "get_other_language".to_string(),
        rust_path: "ts_pack::get_other_language".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let config = capsule_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen.content.contains("typedef struct TSLanguage TSLanguage;"),
        "a capsule type still returned by at least one function must keep its forward \
         declaration, got cbindgen.toml:\n{}",
        cbindgen.content
    );
}
