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
                    original_type: None,
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
                    original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        config_type.content.contains("public sealed record Config"),
        "Should define Config sealed record"
    );

    let enum_type = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("OcrBackend.cs"))
        .unwrap();
    assert!(
        enum_type.content.contains("public enum OcrBackend"),
        "Should define OcrBackend enum"
    );
    // Sanity-check the XML doc summary renders across separate /// lines for
    // both the enum class and its variants — regression guard for the issue
    // where {%- / -%} trimming collapsed the block onto one line.
    assert!(
        enum_type
            .content
            .contains("/// <summary>\n/// Available OCR backends\n/// </summary>"),
        "Enum class doc summary should be on separate /// lines:\n{}",
        enum_type.content
    );
    assert!(
        enum_type
            .content
            .contains("    /// <summary>\n    /// Tesseract OCR\n    /// </summary>"),
        "Enum variant doc summary should be on separate /// lines:\n{}",
        enum_type.content
    );
}

/// Regression: enum XML doc summary used to render on a single line as
/// `/// <summary>/// text/// </summary>` because the jinja `for` loop tags used
/// `{%-` / `-%}` whitespace trimming, eating the newlines after each `///` line.
/// The fix splits the block into three separate `///` lines (and one per doc
/// line + per variant doc line) so the output parses as proper C# XML docs.
#[test]
fn test_enum_doc_summary_emits_separate_lines_for_class_and_variants() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "testlib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "BrowserWait".to_string(),
            rust_path: "testlib::BrowserWait".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "NetworkIdle".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Wait until network activity is idle.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Selector".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Wait for a specific CSS selector to appear in the DOM.\nSecond line of variant doc."
                        .to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Wait strategy for browser page rendering.\nSecond line of enum doc.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = make_config("testlib", Some("Testlib"), true);
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("BrowserWait.cs"))
        .expect("BrowserWait.cs should be generated");
    let content = &enum_file.content;

    // Class-level doc: 3-line summary block, one /// per doc line, separate
    // open/close tags. The concatenated buggy form `<summary>/// text` must
    // never appear.
    assert!(
        !content.contains("<summary>///"),
        "Concatenated summary/doc-line marker should not appear in enum class doc:\n{content}"
    );
    assert!(
        !content.contains("///</summary>") && !content.contains(".</summary>"),
        "Concatenated doc-line/summary close marker should not appear in enum class doc:\n{content}"
    );
    assert!(
        content.contains("/// <summary>\n/// Wait strategy for browser page rendering.\n/// Second line of enum doc.\n/// </summary>\n"),
        "Enum class doc summary should render across separate lines, one /// per source line:\n{content}"
    );

    // Variant-level doc: indented 4 spaces, same shape.
    assert!(
        content.contains("    /// <summary>\n    /// Wait until network activity is idle.\n    /// </summary>\n"),
        "Variant doc summary (NetworkIdle) should render across separate /// lines:\n{content}"
    );
    assert!(
        content.contains("    /// <summary>\n    /// Wait for a specific CSS selector to appear in the DOM.\n    /// Second line of variant doc.\n    /// </summary>\n"),
        "Multi-line variant doc (Selector) should emit one /// per source line:\n{content}"
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                    original_type: None,
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
                    original_type: None,
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
                    original_type: None,
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
                    original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                    original_type: None,
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
                    original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                    original_type: None,
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
                    original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
                original_type: None,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_trait_names: ::std::collections::HashSet::new(),
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

/// D6: Non-nullable reference property without default should emit `required` modifier.
#[test]
fn test_non_nullable_string_field_emits_required() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ServerConfig".to_string(),
            rust_path: "test::ServerConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "host".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "Server hostname".to_string(),
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
                original_type: None,
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ServerConfig.cs"))
        .expect("ServerConfig.cs should be generated");

    assert!(
        cs_file.content.contains("required string Host"),
        "Non-nullable string field without default must emit 'required'; got:\n{}",
        cs_file.content
    );
    assert!(
        !cs_file.content.contains("Host { get; set; } ="),
        "Non-nullable string field must NOT emit default initializer; got:\n{}",
        cs_file.content
    );
}

/// D6: Nullable field should NOT emit `required` modifier.
#[test]
fn test_nullable_field_does_not_emit_required() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
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
                original_type: None,
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Config.cs"))
        .expect("Config.cs should be generated");

    assert!(
        !cs_file.content.contains("required string? Timeout"),
        "Nullable field must NOT emit 'required'; got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("string? Timeout { get; init; } = null"),
        "Nullable field should have null default; got:\n{}",
        cs_file.content
    );
}

/// D6: Collection field should NOT emit `required` modifier.
#[test]
fn test_collection_field_does_not_emit_required() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "cors_origins".to_string(),
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
                original_type: None,
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Config.cs"))
        .expect("Config.cs should be generated");

    assert!(
        !cs_file.content.contains("required List<string> CorsOrigins"),
        "Collection field must NOT emit 'required'; got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("List<string> CorsOrigins { get; init; } = []"),
        "Collection field should have empty collection default; got:\n{}",
        cs_file.content
    );
}

/// D6: Field with explicit default should NOT emit `required`.
#[test]
fn test_field_with_default_does_not_emit_required() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "host".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: Some("127.0.0.1".to_string()),
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: Some(DefaultValue::StringLiteral("127.0.0.1".to_string())),
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = minimal_csharp_config("test");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Config.cs"))
        .expect("Config.cs should be generated");

    assert!(
        !cs_file.content.contains("required string Host"),
        "Field with explicit default must NOT emit 'required'; got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("string Host { get; init; } = \"127.0.0.1\""),
        "Field with default should emit the default value; got:\n{}",
        cs_file.content
    );
}

/// D7: Opaque handle wrapper should use `internal` IntPtr Handle, not `public`.
#[test]
fn test_opaque_handle_wrapper_has_internal_handle() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Document".to_string(),
            rust_path: "test::Document".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "text".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Get document text".to_string(),
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
            doc: "Document handle".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();

    let doc_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Document.cs"))
        .expect("Document.cs should be generated");

    assert!(
        doc_file.content.contains("internal IntPtr Handle =>"),
        "Opaque handle wrapper should use 'internal IntPtr Handle'; got:\n{}",
        doc_file.content
    );
    assert!(
        !doc_file.content.contains("public IntPtr Handle"),
        "Opaque handle wrapper must NOT expose 'public IntPtr Handle'; got:\n{}",
        doc_file.content
    );
}

/// B5: Every generated `.cs` file must use file-scoped namespace syntax (`namespace Foo;`),
/// not block-scoped (`namespace Foo { ... }`).
#[test]
fn test_file_scoped_namespace_emitted() {
    let backend = CsharpBackend;
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_config("test", Some("MyNs"), false);
    let files = backend.generate_bindings(&api, &config).unwrap();
    // Only .cs files contain a namespace declaration; skip project/props files.
    let cs_files: Vec<_> = files
        .iter()
        .filter(|f| f.path.extension().and_then(|e| e.to_str()) == Some("cs"))
        .collect();
    assert!(!cs_files.is_empty(), "At least one .cs file should be generated");
    for file in &cs_files {
        assert!(
            file.content.contains("namespace MyNs;"),
            "File {} must use file-scoped namespace 'namespace MyNs;'; got:\n{}",
            file.path.display(),
            file.content
        );
        assert!(
            !file.content.contains("namespace MyNs {"),
            "File {} must NOT use block-scoped namespace; got:\n{}",
            file.path.display(),
            file.content
        );
    }
}

/// B5: Streaming method on an opaque handle must return `IAsyncEnumerable<T>`,
/// not `Task<List<T>>` or `IEnumerable<T>`.
///
/// The streaming code path is activated only when the crate's adapter list contains an entry
/// with `pattern = "streaming"` for the method name and owner type. This test configures a
/// full streaming adapter via TOML to ensure the emitter produces `IAsyncEnumerable<ChatChunk>`.
#[test]
fn test_streaming_method_returns_iasync_enumerable() {
    let backend = CsharpBackend;
    let toml_str = r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
error_style = "last_error"

[crates.csharp]
namespace = "Test"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "StreamClient"
item_type = "ChatChunk"
error_type = "TestError"

[[crates.adapters.params]]
name = "req"
type = "ChatRequest"
"#;
    let cfg: alef_core::config::NewAlefConfig = toml::from_str(toml_str).unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StreamClient".to_string(),
            rust_path: "test::StreamClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "chat_stream".to_string(),
                params: vec![ParamDef {
                    name: "req".to_string(),
                    ty: TypeRef::Named("ChatRequest".to_string()),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::Vec(Box::new(TypeRef::Named("ChatChunk".to_string()))),
                is_async: true,
                is_static: false,
                error_type: Some("TestError".to_string()),
                doc: "Stream chat completions.".to_string(),
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
            doc: "Streaming client handle.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &config).unwrap();
    let client_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("StreamClient.cs"))
        .expect("StreamClient.cs should be generated");
    assert!(
        client_file.content.contains("IAsyncEnumerable<"),
        "Streaming method must return IAsyncEnumerable<T>; got:\n{}",
        client_file.content
    );
    assert!(
        !client_file.content.contains("Task<List<"),
        "Streaming method must NOT return Task<List<T>>; got:\n{}",
        client_file.content
    );
}

/// B5: `byte[]` fields without an explicit default should use the C# 12 collection
/// expression `= []` instead of `= Array.Empty<byte>()`.
#[test]
fn test_bytes_field_default_uses_collection_expression() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "BlobPayload".to_string(),
            rust_path: "test::BlobPayload".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "data".to_string(),
                ty: TypeRef::Bytes,
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
                original_type: None,
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &config).unwrap();
    let cs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("BlobPayload.cs"))
        .expect("BlobPayload.cs should be generated");
    assert!(
        !cs_file.content.contains("Array.Empty<byte>()"),
        "byte[] default must NOT use Array.Empty<byte>(); got:\n{}",
        cs_file.content
    );
    assert!(
        cs_file.content.contains("= []"),
        "byte[] default must use collection expression '= []'; got:\n{}",
        cs_file.content
    );
}

/// B6: Consecutive using directives must each be on their own line, not concatenated.
/// Regression test for issue where `using System.Runtime.InteropServices;using System.Text.Json;`
/// appeared on a single line instead of separate lines.
#[test]
fn test_using_directives_each_on_own_line() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Parser".to_string(),
            rust_path: "test::Parser".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "parse".to_string(),
                params: vec![],
                return_type: TypeRef::String,
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
                has_default_impl: false,
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
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let files = backend.generate_bindings(&api, &config).unwrap();
    let parser_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Parser.cs"))
        .expect("Parser.cs should be generated");

    // Extract the using directives section (before namespace declaration)
    let content = &parser_file.content;
    let using_section = content
        .lines()
        .take_while(|line| !line.contains("namespace"))
        .collect::<Vec<_>>()
        .join("\n");

    // Check that no two using directives are concatenated on a single line
    for line in using_section.lines() {
        let using_count = line.matches("using ").count();
        assert!(
            using_count <= 1,
            "Each line must contain at most one 'using' directive, but found {} in: {}",
            using_count,
            line
        );
    }
}
