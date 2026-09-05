use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

// -----------------------------------------------------------------------
// -----------------------------------------------------------------------

/// Build an ApiSurface with an opaque type that has a static `new` constructor.
fn opaque_with_constructor_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: "HTTP method enum.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
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
                    cfg: None,
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: "Opaque route builder.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
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
            serde_content: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
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

    assert!(
        lib.content.contains("method: i32"),
        "expected enum parameter 'method: i32', got:\n{}",
        lib.content
    );
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

    assert!(
        lib.content.contains("method_from_i32"),
        "constructor should use method_from_i32 to reconstruct enum from discriminant"
    );
}

#[test]
fn opaque_constructor_enum_failure_uses_scalar_handle_sentinel_and_compiles() {
    let api = opaque_with_constructor_api();
    let files = FfiBackend.generate_bindings(&api, &sample_config()).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let function = lib
        .content
        .split("pub unsafe extern \"C\" fn my_lib_route_builder_new")
        .nth(1)
        .expect("opaque constructor")
        .split("let path_rs")
        .next()
        .expect("enum conversion prefix");

    assert!(function.contains("return 0;"), "{function}");
    assert!(!function.contains("std::ptr::null_mut()"), "{function}");

    let source = format!(
        r#"
type AlefHandle = u64;
fn method_from_i32_rs(value: i32) -> Option<i32> {{ (value == 0).then_some(value) }}
fn set_last_error(_: i32, _: &str) {{}}
fn convert(method: i32) -> AlefHandle {{
{}
    method_rs as AlefHandle
}}
fn main() {{ assert_eq!(convert(1), 0); }}
"#,
        function
            .split("let method_rs")
            .nth(1)
            .map(|body| format!("    let method_rs{body}"))
            .expect("enum conversion body")
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("scalar_handle_constructor.rs");
    let binary_path = directory.path().join("scalar-handle-constructor-test");
    std::fs::write(&source_path, source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
}

#[test]
fn test_opaque_constructor_returns_generational_handle() {
    let api = opaque_with_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let has_handle_return = lib.content.lines().any(|line| line.contains(") -> AlefHandle"));
    assert!(
        has_handle_return,
        "constructor should return a generational handle; got:\n{}",
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

    assert!(
        lib.content.contains("my_lib_route_builder_new"),
        "RouteBuilder (opaque) should have _new constructor"
    );
}

/// Build an ApiSurface with an opaque type whose sole constructor is named
/// `compile` (not `new`) and is fallible (`error_type` is set, representing
/// `Result<Self, E>`).  This mirrors a real crate's `MetaSchema::compile`.
fn opaque_with_named_constructor_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Schema".to_string(),
            rust_path: "my_lib::Schema".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "compile".to_string(),
                params: vec![ParamDef {
                    name: "json_text".to_string(),
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
                return_type: TypeRef::Named("Schema".to_string()),
                is_async: false,
                is_static: true,
                error_type: Some("SchemaError".to_string()),
                doc: "Compile the given JSON text as a schema.".to_string(),
                receiver: None,
                cfg: None,
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "Opaque compiled schema.".to_string(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// A named static constructor (`compile`) on an opaque type must emit a C
/// export whose symbol is `{prefix}_{type_snake}_compile`, NOT `_new`.
#[test]
fn test_named_static_constructor_emits_compile_symbol() {
    let api = opaque_with_named_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn my_lib_schema_compile("),
        "named static constructor must emit _compile symbol, got:\n{}",
        lib.content
    );
}

/// The compile symbol must NOT produce a `_new` alias — the C export name
/// must faithfully reflect the Rust method name.
#[test]
fn test_named_static_constructor_does_not_emit_new_symbol() {
    let api = opaque_with_named_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("pub unsafe extern \"C\" fn my_lib_schema_new("),
        "named static constructor must NOT emit a _new symbol, got:\n{}",
        lib.content
    );
}

/// A fallible named constructor (`error_type` set) must clear the thread-local
/// error state at entry (`clear_last_error`) and propagate errors by calling
/// `set_last_error` + returning a zero handle rather than panicking.
#[test]
fn test_named_static_constructor_is_fallible() {
    let api = opaque_with_named_constructor_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("clear_last_error"),
        "fallible constructor must call clear_last_error(); got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("set_last_error"),
        "fallible constructor must call set_last_error() on error; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("Err(e) =>")
            && lib
                .content
                .contains("set_last_error(1, &e.to_string());\n            0"),
        "fallible constructor must return a zero handle on error; got:\n{}",
        lib.content
    );
}
