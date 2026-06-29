use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// Verify that methods with generic type parameters are skipped from C FFI wrapper generation.
/// Generic methods (like `App::route<H: Handler>(...)`) cannot be wrapped as C functions
/// because generic type parameters have no C FFI representation. These methods are handled
/// through the service-API registration path instead.
#[test]
fn test_skips_method_with_generic_type_parameter() {
    // Create an API with a type that has a method with a generic parameter
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "App".to_string(),
            rust_path: "my_lib::App".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "route".to_string(),
                params: vec![
                    ParamDef {
                        name: "builder".to_string(),
                        ty: TypeRef::Named("RouteBuilder".to_string()),
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
                        name: "handler".to_string(),
                        // This is a generic type parameter H that won't be in path_map
                        ty: TypeRef::Named("H".to_string()),
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
                return_type: TypeRef::Named("App".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Register a handler.".to_string(),
                receiver: Some(ReceiverKind::Owned),
                sanitized: false,
                trait_source: None,
                returns_ref: true,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
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
            doc: "App service.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The method with generic parameter H should NOT be wrapped as a C function
    assert!(
        !lib.content.contains("my_lib_app_route"),
        "method with generic type parameter H should NOT be wrapped as C function"
    );
}

/// Verify that methods returning a reference to the receiver (builder-style methods)
/// are skipped from C FFI wrapper generation. Methods returning `&mut Self` or `&Self`
/// cannot be represented as owned C handles, so they must be accessed through the
/// service-API registration path instead.
#[test]
fn test_skips_method_with_receiver_reference_return() {
    // Create an API with a type that has a builder-style method
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Builder".to_string(),
            rust_path: "my_lib::Builder".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "with_option".to_string(),
                params: vec![ParamDef {
                    name: "value".to_string(),
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
                // This method returns &mut Self (a reference to the receiver)
                return_type: TypeRef::Named("Builder".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Set an option (builder style).".to_string(),
                receiver: Some(ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: true, // Marks that it returns a reference
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
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
            doc: "Builder type.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The builder-style method returning &mut Self should NOT be wrapped as a C function
    assert!(
        !lib.content.contains("my_lib_builder_with_option"),
        "builder-style method returning &mut Self should NOT be wrapped as C function"
    );
}
