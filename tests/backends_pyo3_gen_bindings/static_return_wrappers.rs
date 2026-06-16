use super::*;

/// Regression test: a non-opaque struct with a static `default()` method that returns
/// `TypeRef::Named` with the same name as the struct must wrap the core call with `.into()`.
///
/// Before the fix, `wrap_return_with_mutex` had a guard `if n == type_name { expr }` that
/// silently skipped the conversion, producing code like:
///   `fn default() -> ParseOptions { core::ParseOptions::default() }`
/// which fails to compile because the body returns the core type, not the binding wrapper.
#[test]
fn test_static_default_returns_binding_wrapper_not_core_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::options::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
            methods: vec![MethodDef {
                name: "default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("ParseOptions".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: String::new(),
                receiver: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must call the core default() and convert with .into() so the
    // binding wrapper type is returned, not the bare inner core type.
    assert!(
        content.contains("test_lib::options::ParseOptions::default().into()"),
        "static default() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn default:\n{}",
        extract_fn_snippet(content, "fn default")
    );
}

/// Regression test: a static `from_update()` method on a non-opaque struct that takes a
/// `Named` param and returns `TypeRef::Named` with the same struct name must also end with
/// `.into()` so the core return value is converted to the binding wrapper.
#[test]
fn test_static_from_update_returns_binding_wrapper_not_core_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::options::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
                methods: vec![MethodDef {
                    name: "from_update".to_string(),
                    params: vec![ParamDef {
                        name: "update".to_string(),
                        ty: TypeRef::Named("ParseOptionsUpdate".to_string()),
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
                        core_wrapper: alef::core::ir::CoreWrapper::None,
                    }],
                    return_type: TypeRef::Named("ParseOptions".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: String::new(),
                    receiver: None,
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ParseOptionsUpdate".to_string(),
                rust_path: "test_lib::ParseOptionsUpdate".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field(
                    "enabled",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                    true,
                )],
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
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must delegate to the core method and convert the result with .into().
    assert!(
        content.contains("ParseOptions::from_update(update_core).into()"),
        "static from_update() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn from_update:\n{}",
        extract_fn_snippet(content, "fn from_update")
    );
}

/// Extract a ~200-char snippet around the first occurrence of `marker` for assertion messages.
fn extract_fn_snippet<'a>(content: &'a str, marker: &str) -> &'a str {
    let start = content.find(marker).unwrap_or(0);
    let end = (start + 200).min(content.len());
    &content[start..end]
}
