use alef_backend_csharp::CsharpBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, DefaultValue, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

#[test]
fn test_basic_generation() {
    let backend = CsharpBackend;

    // Create test API surface
    let api = ApiSurface {
        crate_name: "kreuzberg".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "kreuzberg::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "backend".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
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
            doc: "Extraction configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "kreuzberg::extract_file_sync".to_string(),
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
        }],
        enums: vec![EnumDef {
            name: "OcrBackend".to_string(),
            rust_path: "kreuzberg::OcrBackend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Tesseract OCR".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "PaddleOCR backend".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Available OCR backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    // Create test config
    let config = make_config("kreuzberg", Some("Kreuzberg"), true);

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");

    // Check for expected files
    let file_names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();

    assert!(
        file_names.iter().any(|f| f.contains("NativeMethods.cs")),
        "Should generate NativeMethods.cs"
    );
    assert!(
        file_names.iter().any(|f| f.contains("KreuzbergException.cs")),
        "Should generate exception class"
    );
    assert!(
        file_names.iter().any(|f| f.contains("KreuzbergLib.cs")),
        "Should generate wrapper class"
    );
    assert!(
        file_names.iter().any(|f| f.contains("Config.cs")),
        "Should generate Config type"
    );
    assert!(
        file_names.iter().any(|f| f.contains("OcrBackend.cs")),
        "Should generate OcrBackend enum"
    );

    // Verify content of a generated file
    let native_methods = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .unwrap();
    assert!(native_methods.content.contains("DllImport"), "Should contain DllImport");
    assert!(
        native_methods.content.contains("NativeMethods"),
        "Should define NativeMethods class"
    );
    assert!(
        native_methods.content.contains("kreuzberg_ffi"),
        "Should reference kreuzberg_ffi library"
    );

    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("KreuzbergLib.cs"))
        .unwrap();
    assert!(
        wrapper.content.contains("public static class KreuzbergLib"),
        "Should define wrapper class"
    );
    assert!(
        wrapper.content.contains("ExtractFileSync"),
        "Should define wrapper method"
    );

    let config_type = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Config.cs"))
        .unwrap();
    assert!(
        config_type.content.contains("public sealed class Config"),
        "Should define Config sealed class"
    );

    let enum_type = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("OcrBackend.cs"))
        .unwrap();
    assert!(
        enum_type.content.contains("public enum OcrBackend"),
        "Should define OcrBackend enum"
    );
}

#[test]
fn test_ffi_excluded_types_are_not_generated_for_pinvoke() {
    let backend = CsharpBackend;
    let config = make_test_config_with_ffi_excludes("HiddenHandle");
    let hidden_type = TypeDef {
        name: "HiddenHandle".to_string(),
        rust_path: "test_lib::HiddenHandle".to_string(),
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
        doc: "Hidden FFI handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };
    let visible_type = TypeDef {
        name: "VisibleHandle".to_string(),
        rust_path: "test_lib::VisibleHandle".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "hidden".to_string(),
            params: vec![],
            return_type: TypeRef::Named("HiddenHandle".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Returns the hidden handle.".to_string(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            trait_source: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
        doc: "Visible FFI handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![hidden_type, visible_type],
        functions: vec![FunctionDef {
            name: "hidden_handle".to_string(),
            rust_path: "test_lib::hidden_handle".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("HiddenHandle".to_string()),
            is_async: false,
            error_type: None,
            doc: "Returns the hidden handle.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.iter().any(|file| file.path.ends_with("HiddenHandle.cs")));
    assert!(files.iter().any(|file| file.path.ends_with("VisibleHandle.cs")));
    for file in &files {
        assert!(!file.content.contains("HiddenHandle"));
        assert!(!file.content.contains("VisibleHandleHidden"));
    }
}

#[test]
fn test_opaque_method_return_wraps_handle_without_to_json() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GraphQLRouteConfig".to_string(),
            rust_path: "test::GraphQLRouteConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "path".to_string(),
                params: vec![ParamDef {
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
                }],
                return_type: TypeRef::Named("GraphQLRouteConfig".to_string()),
                is_async: false,
                is_static: false,
                error_type: Some("GraphQLError".to_string()),
                doc: "Set the path.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let graph_ql_route_config = files
        .iter()
        .find(|file| file.path.ends_with("GraphQlRouteConfig.cs"))
        .unwrap();

    assert!(
        graph_ql_route_config
            .content
            .contains("var returnValue = new GraphQlRouteConfig(nativeResult);")
    );
    assert!(!graph_ql_route_config.content.contains("GraphQlRouteConfigToJson"));
}

#[test]
fn test_error_helper_preserves_base_error_acronym_class_name() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GraphQLError".to_string(),
            rust_path: "test::GraphQLError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "InvalidInput".to_string(),
                message_template: Some("invalid input: {0}".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: String::new(),
            }],
            doc: String::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files.iter().find(|file| file.path.ends_with("TestLib.cs")).unwrap();

    assert!(
        wrapper
            .content
            .contains("if (code == 2) return new GraphQLErrorException(message);")
    );
    assert!(!wrapper.content.contains("GraphQlErrorException"));
}

#[test]
fn test_namespace_resolution() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("my-lib", Some("MyCompany.MyLib"), false);

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let file_names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();

    // Should contain nested namespace
    assert!(
        file_names.iter().any(|f| f.contains("MyCompany/MyLib")),
        "Should create nested namespace directories"
    );
}

#[test]
fn test_generated_header() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("test", None, false);

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // All files should have generated_header set to true
    for file in &files {
        assert!(
            file.generated_header,
            "All generated files should have generated_header=true"
        );
        assert!(
            file.content.contains("auto-generated"),
            "Content should contain auto-generated marker"
        );
    }
}

#[test]
fn test_type_mapping() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "u32_val".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "i64_val".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I64),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "string_val".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "list_val".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("test", None, false);

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let numbers_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Numbers.cs"))
        .unwrap();
    let content = &numbers_file.content;

    // Verify type mappings
    assert!(content.contains("uint U32Val"), "U32 should map to uint");
    assert!(content.contains("long I64Val"), "I64 should map to long");
    assert!(
        content.contains("string? StringVal"),
        "Optional string should be nullable"
    );
    assert!(
        content.contains("List<string> ListVal"),
        "Vec<String> should map to List<string>"
    );
}

#[test]
fn test_tuple_struct_fields_skipped() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TupleStruct".to_string(),
            rust_path: "test::TupleStruct".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "_1".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("test", None, false);

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let tuple_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TupleStruct.cs"));

    // Types with only tuple fields should not generate a record file at all
    assert!(
        tuple_file.is_none(),
        "Tuple struct with only positional fields should not generate a .cs file"
    );
}

#[test]
fn test_mixed_struct_skips_tuple_fields_only() {
    let backend = CsharpBackend;

    // A struct with both named and tuple fields — only named fields should appear as properties
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MixedStruct".to_string(),
            rust_path: "test::MixedStruct".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("test", None, false);

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let mixed_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("MixedStruct.cs"))
        .expect("MixedStruct.cs should be generated since it has named fields");

    // The named field "label" should appear as a property
    assert!(
        mixed_file.content.contains("Label"),
        "Named field 'label' should generate a property"
    );
    // The tuple field "_0" should NOT appear
    assert!(
        !mixed_file.content.contains("\"_0\""),
        "Tuple field '_0' should not appear in JSON property names"
    );
}

/// Helper: build a ResolvedCrateConfig for C# binding tests.
///
/// - `crate_name`: the crate name (e.g. `"test"`, `"kreuzberg"`)
/// - `namespace`: optional C# namespace override (e.g. `Some("Kreuzberg")`)
/// - `with_ffi`: whether to include FFI config (sets `ffi.prefix = crate_name`)
fn make_config(crate_name: &str, namespace: Option<&str>, with_ffi: bool) -> ResolvedCrateConfig {
    let ns_line = match namespace {
        Some(ns) => format!("namespace = \"{ns}\"\n"),
        None => String::new(),
    };
    let ffi_section = if with_ffi {
        format!("[crates.ffi]\nprefix = \"{crate_name}\"\nerror_style = \"last_error\"\n")
    } else {
        String::new()
    };
    let toml_str = format!(
        "[workspace]\nlanguages = [\"csharp\"]\n[[crates]]\nname = \"{crate_name}\"\nsources = [\"src/lib.rs\"]\n[crates.csharp]\n{ns_line}{ffi_section}",
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_test_config_with_ffi_excludes(excluded_type: &str) -> ResolvedCrateConfig {
    let toml_str = format!(
        r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
exclude_types = ["{excluded_type}"]

[crates.csharp]
namespace = "Test"
"#,
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Helper: build a minimal ResolvedCrateConfig with a CSharp config for crate named "test".
fn minimal_csharp_config(crate_name: &str) -> ResolvedCrateConfig {
    make_config(crate_name, Some("Test"), true)
}

/// Regression test: Duration field in a has_default struct must emit `ulong?` (single `?`),
/// not `ulong??`. Reproduces the CS1519 error introduced by commit 9ee50d0.
#[test]
fn test_duration_field_emits_single_nullable_not_double() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "BrowserConfig".to_string(),
            rust_path: "test::BrowserConfig".to_string(),
            original_rust_path: String::new(),
            // has_default = true triggers the defaulted-field path
            has_default: true,
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Duration,
                optional: false,
                default: None,
                typed_default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("BrowserConfig.cs"))
        .expect("BrowserConfig.cs should be generated");

    // Must have exactly one `?` after ulong — never `??`
    assert!(
        !cs_file.content.contains("ulong??"),
        "Duration field must not produce ulong?? (double nullable); got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("ulong? Timeout"),
        "Duration field should emit `ulong? Timeout`; got:\n{}",
        cs_file.content
    );
}

/// Regression test: Option<ulong> field in a has_default struct must also emit a single `?`.
#[test]
fn test_optional_ulong_field_emits_single_nullable() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "CrawlConfig".to_string(),
            rust_path: "test::CrawlConfig".to_string(),
            original_rust_path: String::new(),
            has_default: true,
            fields: vec![FieldDef {
                name: "max_depth".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                optional: true,
                default: None,
                typed_default: Some(DefaultValue::None),
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("CrawlConfig.cs"))
        .expect("CrawlConfig.cs should be generated");

    assert!(
        !cs_file.content.contains("ulong??"),
        "Optional<ulong> field must not produce ulong?? (double nullable); got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("ulong? MaxDepth"),
        "Optional<ulong> field should emit `ulong? MaxDepth`; got:\n{}",
        cs_file.content
    );
}

/// Regression test: plain enum field with serde(default) and no explicit variant default
/// becomes nullable (T?) with null init — must not double-add `?`.
#[test]
fn test_plain_enum_with_default_emits_single_nullable() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            original_rust_path: String::new(),
            has_default: true,
            fields: vec![FieldDef {
                name: "mode".to_string(),
                ty: TypeRef::Named("Mode".to_string()),
                optional: false,
                default: None,
                // No explicit variant default → default_val will resolve to "null"
                typed_default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Fast".to_string(),
                fields: vec![],
                is_tuple: false,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            }],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Config.cs"))
        .expect("Config.cs should be generated");

    // Should not have double `?` — e.g. `Mode??`
    assert!(
        !cs_file.content.contains("Mode??"),
        "Enum field must not produce Mode?? (double nullable); got:\n{}",
        cs_file.content
    );
    // Should have `Mode? Mode` property (single nullable)
    assert!(
        cs_file.content.contains("Mode?"),
        "Enum field with null default should be nullable; got:\n{}",
        cs_file.content
    );
}

/// Verifies that a function returning `Result<Vec<u8>>` (error_type + TypeRef::Bytes) uses
/// the out-param P/Invoke convention and emits correct wrapper code.
#[test]
fn test_bytes_result_func_emits_out_param_pinvoke_and_wrapper() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "kreuzberg".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_image".to_string(),
            rust_path: "kreuzberg::process_image".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "data".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config("kreuzberg", Some("Kreuzberg"), true);
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation must succeed");

    let native = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .expect("NativeMethods.cs must be generated");

    // P/Invoke: return type must be int (not IntPtr).
    assert!(
        native.content.contains("internal static extern int ProcessImage"),
        "P/Invoke return must be int for bytes_result; got:\n{}",
        native.content
    );
    // P/Invoke: must have input byte-length parameter for byte-slice input.
    assert!(
        native.content.contains("IntPtr data") && native.content.contains("UIntPtr dataLen"),
        "P/Invoke must have byte-slice length parameter; got:\n{}",
        native.content
    );
    // P/Invoke: must have out-params.
    assert!(
        native.content.contains("out IntPtr outPtr"),
        "P/Invoke must have out IntPtr outPtr; got:\n{}",
        native.content
    );
    assert!(
        native.content.contains("out UIntPtr outLen"),
        "P/Invoke must have out UIntPtr outLen; got:\n{}",
        native.content
    );
    assert!(
        native.content.contains("out UIntPtr outCap"),
        "P/Invoke must have out UIntPtr outCap; got:\n{}",
        native.content
    );
    // P/Invoke: FreeBytes declaration must be present.
    assert!(
        native.content.contains("internal static extern void FreeBytes"),
        "NativeMethods.cs must have FreeBytes; got:\n{}",
        native.content
    );

    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("KreuzbergLib.cs"))
        .expect("KreuzbergLib.cs must be generated");

    // Wrapper: return type must be byte[].
    assert!(
        wrapper.content.contains("public static byte[] ProcessImage"),
        "Wrapper return must be byte[] for bytes_result; got:\n{}",
        wrapper.content
    );
    // Wrapper: must pass byte-length argument for byte-slice input.
    assert!(
        wrapper.content.contains("(UIntPtr)data.Length"),
        "Wrapper must pass byte-length argument (UIntPtr)data.Length; got:\n{}",
        wrapper.content
    );
    // Wrapper: must check rc != 0.
    assert!(
        wrapper.content.contains("rc != 0"),
        "Wrapper must check rc != 0; got:\n{}",
        wrapper.content
    );
    // Wrapper: must call Marshal.Copy.
    assert!(
        wrapper.content.contains("Marshal.Copy"),
        "Wrapper must call Marshal.Copy; got:\n{}",
        wrapper.content
    );
    // Wrapper: must call FreeBytes.
    assert!(
        wrapper.content.contains("FreeBytes"),
        "Wrapper must call NativeMethods.FreeBytes; got:\n{}",
        wrapper.content
    );
}
