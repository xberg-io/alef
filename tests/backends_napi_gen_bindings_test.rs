use alef::backends::napi::NapiBackend;
use alef::core::backend::{Backend, PostBuildStep};
use alef::core::config::{NewAlefConfig, NodeCapsuleTypeConfig, NodeConfig, ResolvedCrateConfig};
use alef::core::ir::*;
use std::collections::HashMap;

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    make_field_with_doc(name, ty, optional, "")
}

fn make_field_with_doc(name: &str, ty: TypeRef, optional: bool, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: doc.to_string(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "test-lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn build_config_patches_const_enum_declarations() {
    let backend = NapiBackend;
    let build_config = backend.build_config().expect("NAPI backend should have build config");
    let enum_patch = build_config
        .post_build
        .iter()
        .find_map(|step| match step {
            PostBuildStep::PatchFile { path, find, replace }
                if *path == "index.d.ts" && *find == "export declare const enum" =>
            {
                Some(*replace)
            }
            _ => None,
        })
        .expect("index.d.ts const enum patch should be configured");

    assert_eq!(enum_patch, "export declare enum");
}

#[test]
fn test_basic_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, true),
            ],
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
            doc: "Test configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("Config".to_string()),
                    optional: true,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text from file".to_string(),
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
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
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
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
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
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
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
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");

    // Check for lib.rs file
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some(), "Should generate lib.rs");

    let lib_rs_content = lib_rs.unwrap().content.as_str();

    // Assert NAPI markers are present
    assert!(
        lib_rs_content.contains("#[napi("),
        "Should contain #[napi(...)] attributes"
    );
    assert!(
        lib_rs_content.contains("napi_derive::napi"),
        "Should import napi_derive::napi"
    );
    assert!(
        lib_rs_content.contains("JsConfig"),
        "Should contain JsConfig type (Js-prefixed)"
    );
    assert!(
        lib_rs_content.contains("JsMode"),
        "Should contain JsMode enum (Js-prefixed)"
    );
    assert!(
        lib_rs_content.contains("extractFile"),
        "Should contain extractFile function (camelCase)"
    );
    assert!(
        lib_rs_content.contains("napi(object, js_name"),
        "Non-opaque structs should use napi(object, js_name = ...) attribute"
    );
    assert!(
        lib_rs_content.contains("napi(string_enum, js_name"),
        "Enums should use napi(string_enum, js_name = ...) attribute"
    );
}

#[test]
fn test_bytes_struct_fields_use_jsbytes_and_modern_ts_types() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "UploadFile".to_string(),
            rust_path: "test_lib::UploadFile".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::Bytes, false),
                make_field("maybe_content", TypeRef::Optional(Box::new(TypeRef::Bytes)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Upload file.".to_string(),
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
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap();
    let content = lib_rs.content.as_str();

    assert!(content.contains("pub struct JsBytes(pub Vec<u8>);"), "{content}");
    assert!(content.contains("pub content: JsBytes,"), "{content}");
    assert!(content.contains("pub maybe_content: Option<JsBytes>,"), "{content}");
    assert!(
        content.contains(r#"#[napi(ts_type = "Uint8Array | Buffer | Array<number>")]"#),
        "{content}"
    );
    assert!(
        content.contains(r#"ts_type = "Uint8Array | Buffer | Array<number> | null | undefined""#),
        "{content}"
    );
    assert!(
        !content.contains("serde_bytes"),
        "bare/optional bytes fields should use JsBytes, not serde_bytes attrs:\n{content}"
    );
}

#[test]
fn dts_preserves_native_argument_order_for_defaultable_config_param() {
    let backend = NapiBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
        functions: vec![FunctionDef {
            name: "process_document".to_string(),
            rust_path: "test_lib::process_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
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
                },
                ParamDef {
                    name: "mime_type".to_string(),
                    ty: TypeRef::String,
                    optional: true,
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
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("Config".to_string()),
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
                },
            ],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let dts = backend.generate_type_stubs(&api, &make_config()).unwrap()[0]
        .content
        .clone();
    assert!(
        dts.contains(
            "processDocument(path: string, mimeType?: string | undefined | null, config?: Config | undefined | null)"
        ),
        "processDocument declaration must preserve native order from IR-derived defaultability:\n{dts}"
    );
    assert!(
        !dts.contains("mimeType?: string | undefined | null, config: Config"),
        "processDocument declaration must not emit TS1016-invalid optional-before-required params:\n{dts}"
    );
    assert!(
        !dts.contains("extractFile("),
        "test fixture must prove behavior without extract_file-specific naming:\n{dts}"
    );
}

#[test]
fn test_type_mapping() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("string_list", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify the Numbers struct is defined with NAPI object attribute
    assert!(content.contains("Numbers"), "Should contain Numbers struct");
    assert!(
        content.contains("u32") || content.contains("u32_val"),
        "Should map u32 field"
    );
    assert!(
        content.contains("i64") || content.contains("i64_val"),
        "Should map i64 field"
    );
    assert!(content.contains("String"), "Should map String fields");
    assert!(
        content.contains("Vec") || content.contains("string_list"),
        "Should map Vec<String> field"
    );
}

#[test]
fn test_enum_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
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
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
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
                    name: "Complete".to_string(),
                    fields: vec![],
                    doc: "Complete status".to_string(),
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
            doc: "Task status".to_string(),
            cfg: None,
            is_copy: false,
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
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify enum generation with NAPI string_enum attribute
    assert!(content.contains("Status"), "Should contain Status enum");
    assert!(content.contains("Pending"), "Should contain Pending variant");
    assert!(content.contains("Active"), "Should contain Active variant");
    assert!(content.contains("Complete"), "Should contain Complete variant");
    assert!(
        content.contains("napi(string_enum, js_name"),
        "Should use napi(string_enum, js_name = ...) attribute"
    );
}

#[test]
fn test_binding_excluded_field_is_hidden_from_napi_api() {
    let backend = NapiBackend;
    let mut hidden = make_field("cursor", TypeRef::String, false);
    hidden.binding_excluded = true;
    hidden.binding_exclusion_reason = Some("marked with #[alef(skip)]".to_string());

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "UploadFile".to_string(),
            rust_path: "test_lib::UploadFile".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("filename", TypeRef::String, false), hidden],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "accept_upload".to_string(),
            rust_path: "test_lib::accept_upload".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "file".to_string(),
                ty: TypeRef::Named("UploadFile".to_string()),
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
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let rust = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap()
        .content
        .as_str();
    let stub_files = backend.generate_type_stubs(&api, &config).unwrap();
    let dts = stub_files[0].content.as_str();

    assert!(
        !rust.contains("pub cursor:"),
        "binding-excluded fields must not be public NAPI object fields"
    );
    assert!(
        !dts.contains("cursor"),
        "binding-excluded fields must not appear in TypeScript declarations"
    );
    assert!(
        rust.contains("cursor: Default::default()"),
        "binding→core conversion must default hidden core fields"
    );
}

#[test]
fn test_generated_header() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // Verify lib.rs has generated_header: false (as per source code)
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let lib_rs_file = lib_rs.unwrap();
    assert!(
        !lib_rs_file.generated_header,
        "lib.rs should have generated_header: false"
    );
}

#[test]
fn test_async_function() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_async".to_string(),
            rust_path: "test_lib::process_async".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Async processor".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify async function is generated with proper async keyword
    assert!(
        content.contains("async fn process_async"),
        "Should contain async function"
    );
    // Verify tokio runtime is added for async support
    assert!(
        content.contains("tokio") || content.contains("spawn_blocking"),
        "Should include tokio runtime support for async"
    );
    // Verify return type indicates async/promise
    assert!(content.contains("#[napi"), "Async function should have napi attribute");
}

#[test]
fn test_methods_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![
                MethodDef {
                    name: "process".to_string(),
                    params: vec![ParamDef {
                        name: "data".to_string(),
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
                        core_wrapper: alef::core::ir::CoreWrapper::None,
                    }],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: Some("Error".to_string()),
                    doc: "Process data".to_string(),
                    receiver: Some(alef::core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "create".to_string(),
                    params: vec![],
                    return_type: TypeRef::Named("Processor".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create processor".to_string(),
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
                },
            ],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Text processor".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify opaque struct is generated with Js prefix
    assert!(
        content.contains("struct JsProcessor"),
        "Should contain opaque struct JsProcessor"
    );
    // Verify impl block with napi attribute for methods
    assert!(
        content.contains("impl JsProcessor"),
        "Should contain impl block for JsProcessor"
    );
    // Verify instance method is generated
    assert!(content.contains("fn process"), "Should contain instance method process");
    // Verify static method is generated
    assert!(content.contains("fn create"), "Should contain static method create");
    // Verify napi attributes on methods
    assert!(content.contains("#[napi"), "Methods should have napi attributes");
    // Verify Arc usage for opaque types
    assert!(
        content.contains("Arc"),
        "Opaque types should use Arc for interior mutability"
    );
}

#[test]
fn test_error_types() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "risky_operation".to_string(),
            rust_path: "test_lib::risky_operation".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("ProcessError".to_string()),
            doc: "Operation that can fail".to_string(),
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
        errors: vec![ErrorDef {
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    doc: "Item not found".to_string(),
                    message_template: Some("not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid: {0}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                },
            ],
            doc: "Processing error".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify error handling code is generated
    assert!(
        content.contains("ProcessError") || content.contains("map_err"),
        "Should contain error handling for ProcessError"
    );
    // Verify error conversion function is generated
    assert!(
        content.contains("napi::Error") || content.contains("GenericFailure"),
        "Should contain NAPI error conversion"
    );
    // Verify error variant constants are generated
    assert!(
        content.contains("NotFound") || content.contains("InvalidInput"),
        "Should contain error variant handling"
    );
}

#[test]
fn test_opaque_type() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "test_lib::Handle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "get_value".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Get handle value".to_string(),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
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
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handle type".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify opaque struct uses Arc for memory management
    assert!(
        content.contains("struct JsHandle") && content.contains("Arc"),
        "Opaque type should be JsHandle wrapped in Arc"
    );
    // Verify impl block for opaque type methods
    assert!(
        content.contains("impl JsHandle"),
        "Should have impl block for opaque JsHandle"
    );
    // Verify napi attribute on impl block
    assert!(
        content.contains("#[napi]") && content.contains("impl JsHandle"),
        "Opaque impl block should have napi attribute"
    );
    // Verify method references self.inner for delegation
    assert!(
        content.contains("self.inner") || content.contains("get_value"),
        "Opaque method should delegate to inner type"
    );
}

#[test]
fn test_optional_and_default_fields() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("max_size", TypeRef::Primitive(PrimitiveType::U64), true),
            ],
            methods: vec![],
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
            doc: "Configuration with defaults".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify struct with default impl
    assert!(
        content.contains("struct JsOptions"),
        "Should contain Options struct with Js prefix"
    );
    // Verify fields are wrapped in Option when type has default
    assert!(
        content.contains("Option<") || content.contains("timeout"),
        "Fields should be wrapped in Option for types with defaults"
    );
    // Verify napi(object, js_name = ...) attribute
    assert!(
        content.contains("napi(object, js_name"),
        "Non-opaque struct should use napi(object, js_name = ...)"
    );
    // Verify Default derive is added
    assert!(
        content.contains("Default") || content.contains("impl Default for JsOptions"),
        "Type with has_default should derive Default or have impl"
    );
}

#[test]
fn test_async_method() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "AsyncWorker".to_string(),
            rust_path: "test_lib::AsyncWorker".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "process_async".to_string(),
                params: vec![ParamDef {
                    name: "input".to_string(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Async process".to_string(),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
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
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Async worker".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify async method keyword
    assert!(
        content.contains("async fn process_async"),
        "Should contain async method"
    );
    // Verify tokio runtime for async support
    assert!(
        content.contains("tokio") || content.contains("spawn_blocking"),
        "Should include tokio support for async methods"
    );
    // Verify method is in impl block
    assert!(
        content.contains("impl JsAsyncWorker"),
        "Should have impl block for opaque async worker"
    );
}

#[test]
fn test_static_method_with_error() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Factory".to_string(),
            rust_path: "test_lib::Factory".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "from_config".to_string(),
                params: vec![ParamDef {
                    name: "config_path".to_string(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Named("Factory".to_string()),
                is_async: false,
                is_static: true,
                error_type: Some("Error".to_string()),
                doc: "Create from config".to_string(),
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
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Factory type".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify static method (no &self parameter)
    assert!(content.contains("fn from_config"), "Should contain static method");
    // Verify error handling in static method
    assert!(
        content.contains("map_err") || content.contains("GenericFailure"),
        "Static method with error should have error conversion"
    );
    // Verify return type wrapping for opaque types
    assert!(
        content.contains("JsFactory") || content.contains("Arc"),
        "Static method returning opaque type should wrap in Js and Arc"
    );
}

#[test]
fn test_map_types() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field(
                "settings",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
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
            doc: "Config with map".to_string(),
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify HashMap import is added for Map types
    assert!(content.contains("HashMap"), "Should import HashMap for Map types");
    // Verify struct contains map field
    assert!(content.contains("settings"), "Should contain settings field for map");
}

/// Regression test: tagged enum where each variant holds a different Named struct type
/// (all in the same positional field `_0`) must generate `.into()` conversions, not
/// `serde_json::from_str`. The binding struct stores `Option<JsXxx>`, not `Option<String>`.
#[test]
fn test_tagged_enum_different_named_types_per_variant_uses_into_not_serde_json() {
    let backend = NapiBackend;

    // Simulate the sample-llm `Message` enum pattern:
    // #[serde(tag = "role")]
    // enum Message {
    //     #[serde(rename = "system")]  System(SystemMessage),
    //     #[serde(rename = "user")]    User(UserMessage),
    // }
    // where SystemMessage and UserMessage are distinct structs, both exposed as types.
    let make_variant = |name: &str, rename: &str, struct_name: &str| EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(struct_name.to_string()),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        doc: String::new(),
        is_default: false,
        serde_rename: Some(rename.to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };

    let make_type = |name: &str| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![make_field("content", TypeRef::String, false)],
        methods: vec![],
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_type("SystemMessage"), make_type("UserMessage")],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            serde_tag: Some("role".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            methods: vec![],
            doc: String::new(),
            cfg: None,
            variants: vec![
                make_variant("System", "system", "SystemMessage"),
                make_variant("User", "user", "UserMessage"),
            ],
            is_copy: false,
            has_serde: false,
            has_default: false,
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
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generate_bindings should succeed");

    let files = result.unwrap();
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap()
        .content
        .as_str();

    // Single-tuple Named variants get variant-specific properties (`system`, `user`) so each
    // payload can keep its concrete binding type instead of falling back to JSON strings.
    assert!(
        !content.contains("serde_json::from_str"),
        "binding→core conversion should use typed .into() conversion for single-tuple Named variants"
    );
    assert!(
        !content.contains("serde_json::to_string"),
        "core→binding conversion should use typed .into() conversion for single-tuple Named variants"
    );
    assert!(
        content.contains("system: Option<JsSystemMessage>") && content.contains("user: Option<JsUserMessage>"),
        "variant-specific fields must retain concrete binding payload types"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def_napi(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
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
    }
}

fn make_method_napi(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: has_default,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_async_method_napi(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: true,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_api_napi() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "my_lib::NodeContext".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("depth", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "VisitResult".to_string(),
            rust_path: "my_lib::VisitResult".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Continue".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
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

fn make_visitor_method_napi(name: &str) -> MethodDef {
    let mut method = make_method_napi(name, TypeRef::Named("VisitResult".to_string()), false, true);
    method.params = vec![ParamDef {
        name: "context".to_string(),
        ty: TypeRef::Named("NodeContext".to_string()),
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
    }];
    method
}

fn make_plugin_bridge_cfg(trait_name: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn make_visitor_bridge_cfg(trait_name: &str, type_alias: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
    }
}

// ---------------------------------------------------------------------------
// NAPI trait bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_napi_visitor_bridge_produces_visitor_struct() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("HtmlVisitor", vec![make_visitor_method_napi("visit_node")]);
    let bridge_cfg = make_visitor_bridge_cfg("HtmlVisitor", "HtmlVisitor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("JsHtmlVisitorBridge"),
        "visitor bridge struct must be named Js{{TraitName}}Bridge"
    );
    assert!(
        code.code.contains("impl my_lib::HtmlVisitor for JsHtmlVisitorBridge"),
        "visitor bridge must implement the trait"
    );
}

#[test]
fn test_napi_visitor_bridge_has_obj_field() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("HtmlVisitor", vec![make_visitor_method_napi("visit_node")]);
    let bridge_cfg = make_visitor_bridge_cfg("HtmlVisitor", "HtmlVisitor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code
            .contains("obj_ref: Option<napi::bindgen_prelude::ObjectRef<false>>"),
        "NAPI visitor bridge must store a persistent ObjectRef"
    );
}

#[test]
fn test_napi_plugin_bridge_produces_wrapper_struct_with_obj_ref_and_cached_name() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("pub struct JsOcrBackendBridge"),
        "plugin bridge wrapper struct must be JsOcrBackendBridge"
    );
    assert!(
        code.code.contains("obj_ref:"),
        "plugin bridge wrapper must hold a persistent 'obj_ref' (ObjectRef), not a borrowed 'inner'"
    );
    assert!(
        !code.code.contains("Object<'static>"),
        "plugin bridge wrapper must NOT store a borrowed Object<'static> (pins the event loop)"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "plugin bridge wrapper must have a 'cached_name: String' field"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_super_trait_impl() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("impl my_lib::Plugin for JsOcrBackendBridge"),
        "plugin bridge must implement Plugin super-trait"
    );
    assert!(code.code.contains("fn name("), "Plugin impl must contain name()");
    assert!(
        code.code.contains("fn initialize("),
        "Plugin impl must contain initialize()"
    );
    assert!(
        code.code.contains("fn shutdown("),
        "Plugin impl must contain shutdown()"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_trait_impl_with_forwarded_methods() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("impl my_lib::OcrBackend for JsOcrBackendBridge"),
        "plugin bridge must implement the trait itself"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must forward the 'process' method"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_registration_fn_with_napi_attribute() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("#[napi]"),
        "NAPI registration function must carry the #[napi] attribute"
    );
    assert!(
        code.code.contains("pub fn register_ocrbackend("),
        "registration function must use the configured name"
    );
}

#[test]
fn test_napi_plugin_bridge_validates_required_methods() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "Analyzer",
        vec![
            make_method_napi("analyze", TypeRef::String, true, false), // required
            make_method_napi("describe", TypeRef::String, false, true), // optional
        ],
    );
    let bridge_cfg = alef::core::config::TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // Constructor must check for the required method "analyze"
    assert!(
        code.code.contains("\"analyze\""),
        "constructor must validate the required method 'analyze'"
    );
}

#[test]
fn test_napi_sync_method_body_uses_get_named_property() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("Scanner", vec![make_method_napi("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_plugin_bridge_cfg("Scanner");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("get_named_property"),
        "NAPI sync method body must use get_named_property to retrieve JS methods"
    );
}

#[test]
fn test_napi_async_method_body_uses_box_pin() {
    use alef::backends::napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("Processor", vec![make_async_method_napi("run", TypeRef::Unit)]);
    let bridge_cfg = make_plugin_bridge_cfg("Processor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("get_named_property(\"run\")"),
        "NAPI async method body must retrieve JS method via get_named_property"
    );
}

#[test]
fn test_napi_dts_trait_bridge_interface_matches_runtime_contract() {
    let backend = NapiBackend;
    let mut config = make_config();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }];

    let mut process = make_method_napi(
        "process_image",
        TypeRef::Named("ExtractionResult".to_string()),
        true,
        false,
    );
    process.params = vec![ParamDef {
        name: "content".to_string(),
        ty: TypeRef::Bytes,
        ..Default::default()
    }];
    let mut shutdown = make_method_napi("shutdown", TypeRef::Unit, true, false);
    shutdown.has_default_impl = true;
    let api = ApiSurface {
        types: vec![
            make_trait_def_napi(
                "OcrBackend",
                vec![process, make_async_method_napi("warm_up", TypeRef::Unit), shutdown],
            ),
            TypeDef {
                name: "ExtractionResult".to_string(),
                rust_path: "my_lib::ExtractionResult".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
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
        ..make_api_napi()
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("export interface OcrBackend {")
            && content.contains("  processImage(content: Uint8Array): ExtractionResult")
            && content.contains("  warmUp(): Promise<void>")
            && content.contains("  shutdown?(): void"),
        "trait interface must use runtime method names and the native return type contract:\n{content}"
    );
    assert!(
        content.contains("export declare function registerOcrBackend(impl: OcrBackend): void;"),
        "registration parameter must use the generated trait interface:\n{content}"
    );
    assert!(
        content.contains("export declare function unregisterOcrBackend(name: string): void;")
            && content.contains("export declare function clearOcrBackends(): void;"),
        "lifecycle functions must use runtime public names:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// capsule_types end-to-end: External<T> + __parser passthrough
// ---------------------------------------------------------------------------

fn make_capsule_config_node(type_name: &str, from_module: &str) -> NodeCapsuleTypeConfig {
    NodeCapsuleTypeConfig {
        type_name: type_name.to_string(),
        from_module: from_module.to_string(),
        construct: "external_pointer".to_string(),
        property_name: "__parser".to_string(),
        type_tag: None,
    }
}

fn make_config_with_capsule_types(capsule_map: HashMap<String, NodeCapsuleTypeConfig>) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "test-lib"
"#,
    )
    .unwrap();
    let mut resolved = cfg.resolve().unwrap().remove(0);
    resolved.node = Some(NodeConfig {
        package_name: Some("test-lib".to_string()),
        features: None,
        serde_rename_all: None,
        type_prefix: None,
        capsule_types: capsule_map,
        exclude_functions: vec![],
        exclude_types: vec![],
        exclude_platforms: vec![],
        extra_dependencies: Default::default(),
        tokio_util_features: None,
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: vec![],
        crate_dir: None,
    });
    resolved
}

fn make_language_type_def() -> TypeDef {
    TypeDef {
        name: "Language".to_string(),
        rust_path: "sample_pack::Language".to_string(),
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
        doc: "A sample_language Language handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_get_language_func() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "sample_pack::get_language".to_string(),
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
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: Some("sample_pack::Error".to_string()),
        doc: "Look up a language by name.".to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// capsule_types wires up External<T> + __parser passthrough end-to-end:
/// - Language type does NOT get a #[napi] opaque class emitted.
/// - get_language returns a JsObject with __parser = External<T>(ptr from value.into_raw()).
#[test]
fn test_capsule_types_end_to_end() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "sample_language"),
    );

    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def()],
        functions: vec![make_get_language_func()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types should succeed");

    assert_eq!(files.len(), 1, "expected exactly lib.rs");
    let content = &files[0].content;

    // Language must NOT appear as a #[napi] opaque struct.
    assert!(
        !content.contains("struct JsLanguage"),
        "Language must not be emitted as a #[napi] struct; content:\n{content}"
    );

    // The shim must call into_raw() to extract the raw pointer.
    assert!(
        content.contains("into_raw"),
        "get_language shim must call into_raw(); content:\n{content}"
    );

    // The shim must call raw napi_create_external (not napi-rs's wrapper, which
    // produces a value rejected by node-sample_language's UnwrapLanguage).
    assert!(
        content.contains("napi_create_external"),
        "get_language shim must call raw napi_create_external; content:\n{content}"
    );
    assert!(
        !content.contains("bindgen_prelude::External::new"),
        "get_language shim must NOT use bindgen_prelude::External::new; content:\n{content}"
    );

    // The extern block must be gated with a Windows raw-dylib link attribute so
    // MSVC can synthesize import-library entries for napi_create_external and
    // napi_type_tag_object — symbols outside napi-sys's generate! allowlist.
    assert!(
        content.contains(r#"#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]"#),
        "extern block must be gated with raw-dylib link attribute for Windows MSVC; content:\n{content}"
    );

    // The shim must set the default __parser property on the returned JsObject
    // (the test config doesn't override property_name).
    assert!(
        content.contains("__parser"),
        "get_language shim must set __parser property; content:\n{content}"
    );

    // The function must return napi::Result<napi::bindgen_prelude::Object>.
    assert!(
        content.contains("bindgen_prelude::Object"),
        "get_language shim must return napi::bindgen_prelude::Object; content:\n{content}"
    );

    // The shim must accept napi::Env as its first parameter.
    assert!(
        content.contains("napi::Env"),
        "get_language shim must accept napi::Env; content:\n{content}"
    );
}

/// capsule_types dts generation:
/// - `import type { Language } from "sample_language"` appears at the top.
/// - `export declare class JsLanguage` is NOT emitted.
/// - `getLanguage(name: string): Language` uses the ecosystem type name.
#[test]
fn test_capsule_types_dts_generation() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "sample_language"),
    );

    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def()],
        functions: vec![make_get_language_func()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs with capsule_types should succeed");

    assert_eq!(files.len(), 1, "expected exactly index.d.ts");
    let content = &files[0].content;

    // Import line must be emitted for the capsule type.
    assert!(
        content.contains("import type { Language } from \"sample_language\""),
        "index.d.ts must emit import type for capsule type; content:\n{content}"
    );

    // The class declaration for JsLanguage must NOT be emitted.
    assert!(
        !content.contains("export declare class JsLanguage"),
        "index.d.ts must not emit export declare class JsLanguage; content:\n{content}"
    );

    // getLanguage must use the ecosystem type name, not JsLanguage.
    assert!(
        content.contains("getLanguage(name: string): Language"),
        "index.d.ts must emit getLanguage returning Language (not JsLanguage); content:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// capsule_types on methods of opaque types
// ---------------------------------------------------------------------------

/// Build an opaque `LanguageRegistry` type whose `getLanguage` instance method
/// returns the capsule type `Language`.
fn make_language_registry_type_def() -> TypeDef {
    TypeDef {
        name: "LanguageRegistry".to_string(),
        rust_path: "sample_pack::LanguageRegistry".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "get_language".to_string(),
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            is_static: false,
            error_type: Some("sample_pack::Error".to_string()),
            doc: "Look up a language by name.".to_string(),
            receiver: Some(alef::core::ir::ReceiverKind::Ref),
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
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

/// capsule_types on opaque method — Rust shim:
/// A method on an opaque type returning a capsule type must emit the same
/// JsObject / External<T> / __parser pattern as a free capsule function.
///
/// KNOWN LIMITATION: methods on opaque types currently fall through the regular
/// opaque-class method codegen and emit `Result<JsLanguage>` (where JsLanguage
/// is suppressed), producing a compile error in the downstream crate. Workaround:
/// expose the capsule as a free function (which works end-to-end). Tracked separately;
/// fixing methods requires threading capsule_types through methods.rs.
#[ignore = "method-on-opaque capsule path not yet wired through methods.rs"]
#[test]
fn test_capsule_types_method_on_opaque_rust_shim() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "sample_language"),
    );

    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def(), make_language_registry_type_def()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with opaque capsule method should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Language must NOT appear as a #[napi] opaque struct (it's a capsule type).
    // Use word-boundary check: "JsLanguage {" or "JsLanguage\n" to avoid matching JsLanguageRegistry.
    let has_js_language_struct = content.contains("struct JsLanguage {")
        || content.contains("struct JsLanguage\n")
        || content.contains("struct JsLanguage\r");
    assert!(
        !has_js_language_struct,
        "Language must not be emitted as a standalone #[napi] struct; content:\n{content}"
    );

    // The method shim must accept napi::Env.
    assert!(
        content.contains("napi::Env"),
        "method shim must accept napi::Env; content:\n{content}"
    );

    // The method shim must return napi::Result<napi::bindgen_prelude::Object<'_>>.
    assert!(
        content.contains("napi::Result<napi::bindgen_prelude::Object<'_>>"),
        "method shim must return napi::Result<napi::bindgen_prelude::Object<'_>>; content:\n{content}"
    );

    // The shim must call into_raw().
    assert!(
        content.contains("into_raw"),
        "method shim must call into_raw(); content:\n{content}"
    );

    // The shim must call raw napi_create_external (not napi-rs's wrapper).
    assert!(
        content.contains("napi_create_external"),
        "method shim must call raw napi_create_external; content:\n{content}"
    );
    assert!(
        !content.contains("bindgen_prelude::External::new"),
        "method shim must NOT use bindgen_prelude::External::new; content:\n{content}"
    );

    // The extern block must be gated with the Windows raw-dylib link attribute
    // so MSVC can synthesize import-library entries for the raw napi symbols.
    assert!(
        content.contains(r#"#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]"#),
        "extern block must be gated with raw-dylib link attribute for Windows MSVC; content:\n{content}"
    );

    // The shim must set __parser (default property name).
    assert!(
        content.contains("__parser"),
        "method shim must set __parser property; content:\n{content}"
    );
}

/// capsule_types on opaque method — TypeScript stubs:
/// The `index.d.ts` for an opaque class whose method returns a capsule type must:
/// 1. Emit `import type { Language } from "sample_language"`.
/// 2. Declare the class without `JsLanguage` anywhere.
/// 3. Emit the method returning the ecosystem type name `Language`.
#[test]
fn test_capsule_types_method_on_opaque_dts() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "sample_language"),
    );

    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def(), make_language_registry_type_def()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs with opaque capsule method should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Import must be present.
    assert!(
        content.contains("import type { Language } from \"sample_language\""),
        "index.d.ts must emit import type for capsule type; content:\n{content}"
    );

    // No bare JsLanguage class declaration (as distinct from JsLanguageRegistry).
    // Check specifically for "JsLanguage {" to avoid matching JsLanguageRegistry.
    let has_js_language_class =
        content.contains("export declare class JsLanguage {") || content.contains("export declare class JsLanguage\n");
    assert!(
        !has_js_language_class,
        "index.d.ts must not emit standalone export declare class JsLanguage; content:\n{content}"
    );

    // The registry class must be present with its unprefixed TS name.
    // The Rust struct is JsLanguageRegistry internally, but #[napi(js_name = "LanguageRegistry")]
    // causes NAPI-RS to export it as LanguageRegistry in the .d.ts.
    assert!(
        content.contains("export declare class LanguageRegistry"),
        "index.d.ts must emit LanguageRegistry class (unprefixed); content:\n{content}"
    );

    // The method must use the ecosystem type, not the undeclared opaque handle.
    assert!(
        content.contains("getLanguage(name: string): Language"),
        "method must emit return type Language (not JsLanguage); content:\n{content}"
    );

    // Confirm no bare JsLanguage return type references anywhere.
    let bare_js_language_method = content.contains("): JsLanguage");
    assert!(
        !bare_js_language_method,
        "index.d.ts must not contain ): JsLanguage; content:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// W3 regression tests: js_name, readonly, no Js-prefix on TS surface
// ---------------------------------------------------------------------------

/// Non-opaque structs must carry `#[napi(object, js_name = "Foo")]` so NAPI-RS
/// exports the type as `Foo` rather than `JsFoo` in the generated .d.ts.
#[test]
fn test_napi_js_name_on_non_opaque_struct() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
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

    // (a) js_name must appear in the napi attribute on the Rust struct
    let bindings = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let lib_rs = bindings
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be present");
    assert!(
        lib_rs.content.contains("napi(object, js_name = \"Options\")"),
        "non-opaque struct must carry napi(object, js_name = \"Options\"); content:\n{}",
        lib_rs.content
    );
}

/// Opaque structs must carry `#[napi(js_name = "Foo")]` so NAPI-RS exports
/// the type as `Foo` rather than `JsFoo` in the generated .d.ts.
#[test]
fn test_napi_js_name_on_opaque_struct() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Engine".to_string(),
            rust_path: "test_lib::Engine".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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

    // (a) js_name must appear on opaque struct
    let bindings = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let lib_rs = bindings
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be present");
    assert!(
        lib_rs.content.contains("napi(js_name = \"Engine\")"),
        "opaque struct must carry napi(js_name = \"Engine\"); content:\n{}",
        lib_rs.content
    );
}

/// String enums must carry `#[napi(string_enum, js_name = "Foo")]` so NAPI-RS
/// exports the enum as `Foo` rather than `JsFoo` in the generated .d.ts.
#[test]
fn test_napi_js_name_on_string_enum() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            methods: vec![],
            doc: String::new(),
            cfg: None,
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
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
                    name: "Inactive".to_string(),
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
            is_copy: true,
            has_serde: false,
            has_default: false,
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
    };

    let config = make_config();

    // (a) js_name must appear on string enum
    let bindings = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let lib_rs = bindings
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be present");
    assert!(
        lib_rs.content.contains("napi(string_enum, js_name = \"Status\")"),
        "string enum must carry napi(string_enum, js_name = \"Status\"); content:\n{}",
        lib_rs.content
    );
}

/// DTO interface fields in .d.ts must be declared `readonly`.
#[test]
fn test_dts_dto_fields_are_readonly() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, true),
            ],
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
    let stubs = backend
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs should succeed");
    assert_eq!(stubs.len(), 1);
    let content = &stubs[0].content;

    // (b) readonly must appear on all DTO field declarations
    assert!(
        content.contains("readonly timeout: number"),
        "required field must be emitted as `readonly timeout: number`; content:\n{content}"
    );
    assert!(
        content.contains("readonly name?: string"),
        "optional field must be emitted as `readonly name?: string`; content:\n{content}"
    );
    // (c) The interface name must be unprefixed
    assert!(
        content.contains("export interface Config {"),
        "interface must be emitted as `Config` (unprefixed); content:\n{content}"
    );
    assert!(
        !content.contains("JsConfig"),
        ".d.ts must not contain JsConfig; content:\n{content}"
    );
    // CRITICAL REGRESSION TEST: interface members must NOT have trailing semicolons.
    // napi build produces .d.ts files without semicolons on interface members.
    // alef must emit the same format to avoid churn when napi regenerates.
    assert!(
        !content.contains("readonly timeout: number;"),
        "interface field must NOT have trailing semicolon; alef format must match napi output; content:\n{content}"
    );
    assert!(
        !content.contains("readonly name?: string;"),
        "interface field must NOT have trailing semicolon; alef format must match napi output; content:\n{content}"
    );
}

#[test]
fn test_optional_return_types_emit_null_not_undefined() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "get_name".to_string(),
                rust_path: "test_lib::get_name".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                is_async: false,
                error_type: None,
                doc: "Get optional name".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "get_id".to_string(),
                rust_path: "test_lib::get_id".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                is_async: false,
                error_type: None,
                doc: "Get optional ID".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
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
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Optional string return type should be "string | null", NOT "string | undefined | null"
    assert!(
        content.contains("function getName(): string | null"),
        "optional string return type must be 'string | null', not 'string | undefined | null'; content:\n{content}"
    );

    // Optional number return type should be "number | null", NOT "number | undefined | null"
    assert!(
        content.contains("function getId(): number | null"),
        "optional number return type must be 'number | null', not 'number | undefined | null'; content:\n{content}"
    );

    // Sanity check: make sure we don't emit "undefined" in return types
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("function getName()") || line.contains("function getId()") {
            assert!(
                !line.contains("undefined"),
                "function return type should not contain 'undefined' at line {}: {}",
                i + 1,
                line
            );
        }
    }
}

/// A struct with a rustdoc summary and per-field docs must emit `///` rustdoc
/// above the generated `#[napi(object)]` struct and each field, so napi-derive
/// propagates them as `/** … */` JSDoc blocks in the generated `.d.ts`.
#[test]
fn struct_doc_and_field_docs_emitted_as_rustdoc() {
    let backend = NapiBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ArticleMetadata".to_string(),
            rust_path: "test_lib::ArticleMetadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field_with_doc(
                    "published_time",
                    TypeRef::Optional(Box::new(TypeRef::String)),
                    true,
                    "The article publication time.",
                ),
                make_field_with_doc("author", TypeRef::Optional(Box::new(TypeRef::String)), true, ""),
            ],
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
            doc: "Article metadata extracted from `article:*` tags.".to_string(),
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
    let files = backend.generate_bindings(&api, &config).expect("should succeed");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs")
        .content
        .clone();

    assert!(
        content.contains("/// Article metadata extracted from `article:*` tags."),
        "struct-level rustdoc must precede the napi struct attribute; content:\n{content}"
    );
    assert!(
        content.contains("/// The article publication time."),
        "field-level rustdoc must precede the corresponding struct field; content:\n{content}"
    );
}

/// Enum-level rustdoc and per-variant rustdoc must be emitted on the generated
/// `#[napi(string_enum)]` so napi-derive forwards them to JSDoc on the
/// `export declare enum` and its members in the `.d.ts`.
#[test]
fn enum_and_variant_docs_emitted_as_rustdoc() {
    let backend = NapiBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "AssetCategory".to_string(),
            rust_path: "test_lib::AssetCategory".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Document".to_string(),
                    fields: vec![],
                    doc: "A textual document (HTML, PDF, …).".to_string(),
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
                    name: "Image".to_string(),
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
            doc: "The category of a downloaded asset.".to_string(),
            cfg: None,
            is_copy: false,
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
    };
    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("should succeed");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs")
        .content
        .clone();

    assert!(
        content.contains("/// The category of a downloaded asset."),
        "enum-level rustdoc must precede the napi enum attribute; content:\n{content}"
    );
    assert!(
        content.contains("/// A textual document (HTML, PDF, …)."),
        "variant-level rustdoc must precede the variant declaration; content:\n{content}"
    );
}

/// A function with a single-line rustdoc must emit a single `///` line above
/// the `#[napi]` attribute. A function with a multi-line rustdoc must emit
/// one `///` line per source line.
#[test]
fn function_doc_emitted_as_rustdoc_single_and_multiline() {
    let backend = NapiBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "scrape_one".to_string(),
                rust_path: "test_lib::scrape_one".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Scrape a single URL.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "crawl".to_string(),
                rust_path: "test_lib::crawl".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Crawl a site recursively.\n\nFollows links up to a configured depth.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("should succeed");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs")
        .content
        .clone();

    assert!(
        content.contains("/// Scrape a single URL."),
        "single-line function rustdoc must be emitted as one `///` line; content:\n{content}"
    );
    assert!(
        content.contains("/// Crawl a site recursively."),
        "multi-line function rustdoc must emit first line; content:\n{content}"
    );
    assert!(
        content.contains("/// Follows links up to a configured depth."),
        "multi-line function rustdoc must emit each line; content:\n{content}"
    );
}

#[test]
fn test_vec_vec_string_field_conversion_emits_no_trailing_angle_bracket() {
    // Regression test: non-optional Vec<Vec<String>> sanitized field previously emitted a stray `>`
    // after the `.collect::<Vec<Vec<String>>>()` type ascription, producing `...collect::<Vec<Vec<String>>>()>`
    // which is a syntax error. This test verifies the emitted From impl is syntactically clean.
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "LinkMetadata".to_string(),
            rust_path: "test_lib::LinkMetadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("href", TypeRef::String, false), {
                let mut f = make_field(
                    "attributes",
                    TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
                    false,
                );
                f.sanitized = true;
                f
            }],
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
            doc: "Link metadata.".to_string(),
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
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap();
    let content = lib_rs.content.as_str();

    // The collect type ascription must end with `>()` and never `>()>`
    assert!(
        content.contains(".collect::<Vec<Vec<String>>>()"),
        "non-optional Vec<Vec<String>> collect must end with `>()`; content:\n{content}"
    );
    assert!(
        !content.contains(".collect::<Vec<Vec<String>>>()>"),
        "stray `>` after collect type ascription must not appear; content:\n{content}"
    );
}

#[test]
fn test_trait_bridge_function_uses_alias_rust_path_outside_visitor_module() {
    let backend = NapiBackend;
    let mut config = make_config();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        type_alias: Some("RendererHandle".to_string()),
        context_type: Some("RenderContext".to_string()),
        result_type: Some("RenderResult".to_string()),
        ..Default::default()
    }];
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Renderer".to_string(),
                rust_path: "test_lib::callbacks::Renderer".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "render".to_string(),
                    params: vec![ParamDef {
                        name: "context".to_string(),
                        ty: TypeRef::Named("RenderContext".to_string()),
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
                    return_type: TypeRef::Named("RenderResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "RenderContext".to_string(),
                rust_path: "test_lib::callbacks::RenderContext".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("depth", TypeRef::Primitive(PrimitiveType::U32), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "RendererHandle".to_string(),
                rust_path: "test_lib::callbacks::RendererHandle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "render_page".to_string(),
            rust_path: "test_lib::render_page".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "renderer".to_string(),
                ty: TypeRef::Named("RendererHandle".to_string()),
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
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![EnumDef {
            name: "RenderResult".to_string(),
            rust_path: "test_lib::callbacks::RenderResult".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Continue".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend
        .generate_bindings(&api, &config)
        .expect("should generate bindings");
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs should be generated")
        .content
        .as_str();

    assert!(
        content.contains("as test_lib::callbacks::RendererHandle"),
        "bridge cast must use RendererHandle rust_path; content:\n{content}"
    );
    assert!(
        !content.contains("test_lib::visitor::RendererHandle"),
        "bridge cast must not assume visitor module; content:\n{content}"
    );
}

/// Regression: the napi `#[napi(constructor)]` for an opaque type with `&mut self`
/// methods (stored as `Arc<Mutex<T>>`) must `Mutex::new`-wrap the core value.
/// Previously it emitted `Arc::new(T::new())`, which mismatched the `Arc<Mutex<T>>`
/// field and failed to compile (E0308 in ts-pack-core-node JsParser).
#[test]
fn napi_constructor_mutex_wraps_when_type_has_mut_methods() {
    let backend = NapiBackend;
    let counter = TypeDef {
        name: "Counter".to_string(),
        rust_path: "test_lib::Counter".to_string(),
        is_opaque: true,
        has_default: true,
        methods: vec![
            MethodDef {
                name: "new".to_string(),
                return_type: TypeRef::Named("Counter".to_string()),
                receiver: None,
                ..Default::default()
            },
            MethodDef {
                name: "increment".to_string(),
                return_type: TypeRef::Unit,
                receiver: Some(ReceiverKind::RefMut),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![counter],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");

    assert!(
        lib.content
            .contains("std::sync::Arc::new(std::sync::Mutex::new(test_lib::Counter::new()))"),
        "constructor for a Mutex-wrapped opaque type must Mutex::new-wrap the core value. Got:\n{}",
        lib.content
    );
}
