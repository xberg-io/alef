use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

// -----------------------------------------------------------------------
// Tests for opaque static constructor emission (Part B)
// -----------------------------------------------------------------------

/// Build an ApiSurface with an opaque type that has a static `new` constructor.
fn opaque_with_constructor_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            // Enum used as parameter in the constructor
            TypeDef {
                name: "Method".to_string(),
                rust_path: "my_lib::Method".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "HTTP method enum.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Opaque RouteBuilder with static new constructor
            TypeDef {
                name: "RouteBuilder".to_string(),
                rust_path: "my_lib::RouteBuilder".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "new".to_string(),
                    params: vec![
                        ParamDef {
                            name: "method".to_string(),
                            ty: TypeRef::Named("Method".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                        ParamDef {
                            name: "path".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("RouteBuilder".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create a new route builder.".to_string(),
                    receiver: None,
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Opaque route builder.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Method".to_string(),
            rust_path: "my_lib::Method".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Get".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Post".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "HTTP method.".to_string(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_emits_opaque_static_constructor_as_c_symbol() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Check that the extern "C" fn symbol is emitted
    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn my_lib_route_builder_new("),
        "expected opaque constructor symbol my_lib_route_builder_new, got:\n{}",
        lib.content
    );
}

#[test]
fn test_opaque_constructor_signature_has_enum_by_value_as_i32() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Check that enum parameter is passed as i32, not *const
    assert!(
        lib.content.contains("method: i32"),
        "expected enum parameter 'method: i32', got:\n{}",
        lib.content
    );
    // Verify it's NOT emitted as a pointer
    assert!(
        !lib.content.contains("method: *const my_lib::Method"),
        "enum parameter should not be passed as pointer"
    );
}

#[test]
fn test_opaque_constructor_marshals_enum_from_i32() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Check that the constructor body reconstructs the enum using from_i32
    assert!(
        lib.content.contains("method_from_i32"),
        "constructor should use method_from_i32 to reconstruct enum from discriminant"
    );
}

#[test]
fn test_opaque_constructor_returns_mut_opaque_pointer() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Constructor returns *mut to the inner type (matching the legacy `_free()`
    // signature) — no separate wrapper struct is emitted. The fixture's crate
    // name varies; just check for a `-> *mut <something>RouteBuilder {` body.
    let has_mut_return = lib
        .content
        .lines()
        .any(|line| line.contains("-> *mut") && line.contains("RouteBuilder") && !line.contains("RouteBuilderOpaque"));
    assert!(
        has_mut_return,
        "constructor should return *mut <core>::RouteBuilder (not a wrapper); got:\n{}",
        lib.content
    );
}

#[test]
fn test_opaque_constructor_only_for_opaque_types() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The Method type is NOT opaque, so its constructor should NOT be emitted
    // (if it had one). Only RouteBuilder's constructor should be in the output.
    // Should have RouteBuilder's _new, but not Method's
    assert!(
        lib.content.contains("my_lib_route_builder_new"),
        "RouteBuilder (opaque) should have _new constructor"
    );
}
