use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, DefaultValue, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

#[test]
fn test_basic_generation() {
    let backend = CsharpBackend;

    // Create test API surface
    let api = ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "sample_crate::Config".to_string(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "sample_crate::extract_file_sync".to_string(),
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
            name: "TextBackend".to_string(),
            rust_path: "sample_crate::TextBackend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "PlainText".to_string(),
                    fields: vec![],
                    doc: "Plain text parser".to_string(),
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
                    name: "RichText".to_string(),
                    fields: vec![],
                    doc: "Rich text parser".to_string(),
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
            doc: "Available text backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
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

    // Create test config
    let config = make_config("sample_crate", Some("SampleCrate"), true);

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
        file_names.iter().any(|f| f.contains("SampleCrateException.cs")),
        "Should generate exception class"
    );
    assert!(
        file_names.iter().any(|f| f.contains("SampleCrateConverter.cs")),
        "Should generate wrapper class"
    );
    assert!(
        file_names.iter().any(|f| f.contains("Config.cs")),
        "Should generate Config type"
    );
    assert!(
        file_names.iter().any(|f| f.contains("TextBackend.cs")),
        "Should generate TextBackend enum"
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
        native_methods.content.contains("sample_crate_ffi"),
        "Should reference sample_crate_ffi library"
    );

    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SampleCrateConverter.cs"))
        .unwrap();
    assert!(
        wrapper.content.contains("public static class SampleCrateConverter"),
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
        .find(|f| f.path.to_string_lossy().contains("TextBackend.cs"))
        .unwrap();
    assert!(
        enum_type.content.contains("public enum TextBackend"),
        "Should define TextBackend enum"
    );
    // Sanity-check the XML doc summary renders across separate /// lines for
    // both the enum class and its variants — regression guard for the issue
    // where {%- / -%} trimming collapsed the block onto one line.
    assert!(
        enum_type
            .content
            .contains("/// <summary>\n/// Available text backends\n/// </summary>"),
        "Enum class doc summary should be on separate /// lines:\n{}",
        enum_type.content
    );
    assert!(
        enum_type
            .content
            .contains("    /// <summary>\n    /// Plain text parser\n    /// </summary>"),
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
                    doc: "Wait until network activity is idle.".to_string(),
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
                    name: "Selector".to_string(),
                    fields: vec![],
                    doc: "Wait for a specific CSS selector to appear in the DOM.\nSecond line of variant doc."
                        .to_string(),
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
            doc: "Wait strategy for browser page rendering.\nSecond line of enum doc.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
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
        doc: "Visible FFI handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let graph_ql_route_config = files
        .iter()
        .find(|file| file.path.ends_with("GraphQLRouteConfig.cs"))
        .unwrap();

    assert!(
        graph_ql_route_config
            .content
            .contains("var returnValue = new GraphQLRouteConfig(nativeResult);")
    );
    assert!(!graph_ql_route_config.content.contains("GraphQLRouteConfigToJson"));
}

#[test]
fn test_bool_param_call_site_matches_pinvoke_bool_decl() {
    // The P/Invoke declaration in gen_bindings::functions.rs emits
    // `[MarshalAs(UnmanagedType.U1)] bool <name>` for bool parameters, so the
    // wrapper method must pass the C# `bool` value directly. Previously the
    // call site emitted `(<name> ? 1 : 0)` (an int), which C# rejected with
    // `CS1503: Argument N: cannot convert from 'int' to 'bool'`. Pin both
    // sides of the contract here so they never drift again.
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
                name: "enable_playground".to_string(),
                params: vec![ParamDef {
                    name: "enable".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
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
                return_type: TypeRef::Named("GraphQLRouteConfig".to_string()),
                is_async: false,
                is_static: false,
                error_type: Some("GraphQLError".to_string()),
                doc: "Toggle playground.".to_string(),
                receiver: Some(ReceiverKind::Ref),
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|file| file.path.ends_with("GraphQLRouteConfig.cs"))
        .unwrap();
    let native = files
        .iter()
        .find(|file| file.path.ends_with("NativeMethods.cs"))
        .unwrap();

    // P/Invoke side declares bool with U1 marshalling.
    assert!(
        native.content.contains("[MarshalAs(UnmanagedType.U1)] bool enable"),
        "P/Invoke decl must keep `[MarshalAs(UnmanagedType.U1)] bool` for the enable param; got:\n{}",
        native.content
    );

    // Call site passes the C# bool directly — never the legacy `? 1 : 0` int.
    assert!(
        !wrapper.content.contains("(enable ? 1 : 0)"),
        "Call site must not emit `(enable ? 1 : 0)` (would not type-check against the bool P/Invoke param); got:\n{}",
        wrapper.content
    );
    assert!(
        wrapper
            .content
            .contains("EnablePlayground(\n            Handle,\n            enable\n")
            || wrapper.content.contains("EnablePlayground(Handle, enable"),
        "Call site must pass `enable` directly to the P/Invoke; got:\n{}",
        wrapper.content
    );
}

#[test]
fn test_fallible_unit_opaque_method_checks_last_error_code() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Session".to_string(),
            rust_path: "test::Session".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "close".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: false,
                error_type: Some("SessionError".to_string()),
                doc: "Close the session.".to_string(),
                receiver: Some(ReceiverKind::Ref),
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
            doc: "Session handle.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "SessionError".to_string(),
            rust_path: "test::SessionError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files.iter().find(|file| file.path.ends_with("Session.cs")).unwrap();

    assert!(wrapper.content.contains("NativeMethods.SessionClose("));
    assert!(
        wrapper.content.contains("if (NativeMethods.LastErrorCode() != 0)"),
        "fallible unit methods must preserve FFI errors: {}",
        wrapper.content
    );
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
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|file| file.path.ends_with("TestConverter.cs"))
        .unwrap();

    assert!(
        wrapper
            .content
            .contains("if (code == 2) return new GraphQLErrorException(message);")
    );
    assert!(!wrapper.content.contains("GraphQlErrorException"));
}

/// Regression test for the GraphQLErrorException case in sample_router: rustdoc with
/// `# Examples`, ```ignore code fence, `Self::error_code`, `Result<T, E>` and
/// intra-doc links must not leak verbatim into the generated `<summary>` element.
/// Without sanitisation Roslyn rejected the result with CS1002/CS1519 errors.
#[test]
fn test_error_class_doc_strips_rust_idioms_and_sections() {
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
            variants: vec![],
            doc: "Errors that can occur during GraphQL operations\n\n\
                These errors are compatible with async-graphql error handling.\n"
                .to_string(),
            methods: vec![MethodDef {
                name: "status_code".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::U16),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Convert error to HTTP status code\n\n\
                    Public alias for codes returned by [`Self::error_code`].\n\n\
                    # Examples\n\n\
                    ```ignore\n\
                    use sample_router_graphql::error::GraphQLError;\n\
                    let error = GraphQLError::AuthenticationError(\"x\".to_string());\n\
                    assert_eq!(error.status_code(), 401);\n\
                    ```\n"
                    .to_string(),
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
            }],
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let exception_file = files
        .iter()
        .find(|file| file.path.ends_with("GraphQLErrorException.cs"))
        .expect("GraphQLErrorException.cs must be emitted");
    let content = &exception_file.content;

    // Sentinel rustdoc markup that previously broke Roslyn parsing must be gone.
    assert!(!content.contains("```"), "code fence markers must not leak: {content}");
    assert!(
        !content.contains("# Examples"),
        "section heading must be stripped: {content}"
    );
    assert!(
        !content.contains("Self::error_code"),
        "Self::method path must be normalised: {content}"
    );
    assert!(
        !content.contains("[`"),
        "intra-doc link square brackets must be stripped: {content}"
    );
    assert!(
        !content.contains("GraphQLError::AuthenticationError"),
        "rust code inside fence must be dropped: {content}"
    );
    // The high-level prose survives.
    assert!(
        content.contains("Errors that can occur during GraphQL operations"),
        "base error prose survives: {content}"
    );
    assert!(
        content.contains("Convert error to HTTP status code"),
        "method first line survives: {content}"
    );
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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

/// Regression: when two thiserror enums in the same crate declare variants with
/// the same name (e.g. `GraphQLError::ValidationError` and
/// `SchemaError::ValidationError` in sample_router), the C# backend used to emit two
/// `GeneratedFile` entries sharing the same path
/// (`{VariantName}Exception.cs`). The downstream `write_files` step processes
/// the file list with `rayon::par_iter`, so the two payloads racily overwrite
/// each other — if one payload is longer than the other, the truncate-on-open
/// of the second writer happens before the first writer has flushed all its
/// bytes, leaving a tail of stale bytes past the file's logical closing brace.
///
/// The observable symptom is a corrupted file that contains valid content
/// through the closing `}` followed by garbage like
/// `tring message, Exception innerException) : base(message, innerException) { }\n}\n`
/// — a partial suffix of a constructor line from the other variant's payload.
///
/// The fix dedups by class name: the first occurrence wins, subsequent
/// same-named variants are dropped. This test asserts that:
///   1. No two `GeneratedFile` entries share the same path.
///   2. Each emitted exception file is well-formed: ends with a single closing
///      `}` line and contains no constructor signatures without their full
///      `public {ClassName}(` prefix.
#[test]
fn test_duplicate_variant_names_across_error_enums_do_not_corrupt_files() {
    let backend = CsharpBackend;
    let config = make_config("sample_router", Some("SampleRouter"), true);

    // Two error enums, each declaring a `ValidationError` variant — the exact
    // pattern from sample_router that produced the corruption.
    let make_variant = |name: &str, doc: &str, is_unit: bool| ErrorVariant {
        name: name.to_string(),
        message_template: Some(format!("{}: {{0}}", name.to_lowercase())),
        fields: vec![],
        has_source: false,
        has_from: false,
        is_unit,
        is_tuple: false,
        doc: doc.to_string(),
    };

    let api = ApiSurface {
        crate_name: "sample_router".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![
            ErrorDef {
                name: "GraphQLError".to_string(),
                rust_path: "sample_router::GraphQLError".to_string(),
                original_rust_path: String::new(),
                // Longer doc on GraphQL side — produces a longer payload than
                // the SchemaError side, exposing the truncate-race.
                variants: vec![
                    make_variant(
                        "ValidationError",
                        "GraphQL validation error\n\nOccurs when a GraphQL query fails schema validation.",
                        false,
                    ),
                    make_variant(
                        "DepthLimitExceeded",
                        "Query depth limit exceeded\n\nOccurs when a GraphQL query exceeds the configured depth limit.",
                        true,
                    ),
                ],
                doc: String::new(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            ErrorDef {
                name: "SchemaError".to_string(),
                rust_path: "sample_router::SchemaError".to_string(),
                original_rust_path: String::new(),
                // Shorter doc on SchemaError side — would corrupt the longer
                // GraphQL-side file if both were written to the same path.
                variants: vec![
                    make_variant("ValidationError", "Configuration validation error", false),
                    make_variant("DepthLimitExceeded", "Depth limit exceeded", false),
                ],
                doc: String::new(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).expect("generate ok");

    // (1) No two GeneratedFile entries may share the same path. Without this
    // invariant, write_files' par_iter can leave tails of bytes from the
    // longer payload past the shorter payload's end-of-file.
    let mut seen_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    for file in &files {
        assert!(
            seen_paths.insert(file.path.clone()),
            "duplicate output path `{}` — two GeneratedFile entries write to the same path and will race in write_files",
            file.path.display()
        );
    }

    // (2) Each variant exception file must be well-formed. A pure state-machine
    // walk: the last non-empty line is exactly `}`, and every line that
    // contains the constructor body marker `: base(` is preceded by the full
    // `public {ClassName}(` token on the same line. A truncated leftover line
    // like `tring message, Exception innerException) : base(...) { }` would
    // satisfy the `: base(` check but fail the `public ` prefix check.
    for variant_class in ["ValidationErrorException", "DepthLimitExceededException"] {
        let file_name = format!("{variant_class}.cs");
        let file = files
            .iter()
            .find(|f| f.path.file_name().is_some_and(|n| n == file_name.as_str()))
            .unwrap_or_else(|| panic!("must emit {file_name}"));
        let content = &file.content;
        let non_empty_lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        let last = non_empty_lines.last().copied().unwrap_or("");
        assert_eq!(
            last.trim(),
            "}",
            "{file_name} must end with a single `}}` after closing brace — found `{last}`\nfull content:\n{content}"
        );
        // Count balanced braces — exactly one open class brace, one close.
        let opens = content.matches('{').count();
        let closes = content.matches('}').count();
        assert_eq!(
            opens, closes,
            "{file_name} must have balanced braces (opens={opens}, closes={closes})\ncontent:\n{content}"
        );
        // Every `: base(` line must carry the full `public ` prefix on the same
        // line — guards against truncated constructor leftovers.
        for (idx, line) in content.lines().enumerate() {
            if line.contains(": base(") {
                assert!(
                    line.contains("public "),
                    "{file_name} line {idx} contains `: base(` without the `public ` keyword — \
                     this is the signature of a truncated leftover from a racy write_files pass.\n\
                     line: `{line}`\nfull content:\n{content}"
                );
            }
        }
    }
}

/// Helper: build a ResolvedCrateConfig for C# binding tests.
///
/// - `crate_name`: the crate name (e.g. `"test"`, `"sample_crate"`)
/// - `namespace`: optional C# namespace override (e.g. `Some("SampleCrate")`)
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

#[test]
fn wrapper_functions_cleanup_owned_handles_only_in_finally() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test::ExtractionConfig".to_string(),
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_bytes".to_string(),
            rust_path: "test::extract_bytes".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "content".to_string(),
                    ty: TypeRef::Bytes,
                    optional: false,
                    ..Default::default()
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    optional: true,
                    ..Default::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("TestConverter.cs"))
        .expect("wrapper class should be generated");

    assert_eq!(
        lib.content.matches("contentHandle.Free();").count(),
        1,
        "pinned byte input must be released exactly once:\n{}",
        lib.content
    );
    assert_eq!(
        lib.content.matches("ExtractionConfigFree(configHandle)").count(),
        1,
        "named config handle must be released exactly once:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains(
            "if (configHandle != global::System.IntPtr.Zero) NativeMethods.ExtractionConfigFree(configHandle);"
        ),
        "optional named cleanup must preserve the IntPtr.Zero guard:\n{}",
        lib.content
    );
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Fast".to_string(),
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
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_image".to_string(),
            rust_path: "sample_crate::process_image".to_string(),
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("SampleCrateError".to_string()),
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

    let config = make_config("sample_crate", Some("SampleCrate"), true);
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
        .find(|f| f.path.to_string_lossy().contains("SampleCrateConverter.cs"))
        .expect("SampleCrateConverter.cs must be generated");

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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
            doc: "Document handle".to_string(),
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
    let cfg: alef::core::config::NewAlefConfig = toml::from_str(toml_str).unwrap();
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
            doc: "Streaming client handle.".to_string(),
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

    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .expect("NativeMethods.cs should be generated");
    assert!(
        native_file
            .content
            .contains("EntryPoint = \"test_stream_client_chat_stream_start\""),
        "streaming adapter entry point must use configured owner_type, got:\n{}",
        native_file.content
    );
    assert!(
        native_file
            .content
            .contains("IntPtr StreamClientChatStreamStart(IntPtr client, IntPtr req)"),
        "streaming adapter C# symbol must use configured owner_type, got:\n{}",
        native_file.content
    );
    assert!(
        !native_file.content.contains("CrawlEngineHandleChatStreamStart"),
        "streaming adapter must not emit crawl_engine_handle fallback symbols, got:\n{}",
        native_file.content
    );
}

#[test]
fn test_required_config_param_stays_required() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            fields: vec![FieldDef {
                name: "mode".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            is_clone: true,
            has_serde: true,
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "run".to_string(),
            rust_path: "test::run".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestConverter.cs"))
        .expect("wrapper should be generated");
    assert!(
        wrapper.content.contains("public static void Run(Config config)"),
        "required config must stay required in the public signature, got:\n{}",
        wrapper.content
    );
    assert!(
        wrapper.content.contains("ArgumentNullException.ThrowIfNull(config);"),
        "required config must get a null check, got:\n{}",
        wrapper.content
    );
    assert!(
        !wrapper.content.contains("Config? config"),
        "required config must not be promoted to nullable by name, got:\n{}",
        wrapper.content
    );
    assert!(
        !wrapper.content.contains("config ?? new Config()"),
        "required config must not default by name, got:\n{}",
        wrapper.content
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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

#[test]
fn test_client_constructors_emits_factory_method_and_pinvoke() {
    let toml_str = r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = "MyLib"

[workspace.client_constructors.DefaultClient]
body = "my_lib::DefaultClient::new(api_key)"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_str).unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "my_lib::DefaultClient".to_string(),
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

    let backend = CsharpBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    // Find DefaultClient.cs — should contain the factory method.
    let handle_file = files.iter().find(|f| f.path.ends_with("DefaultClient.cs"));
    assert!(handle_file.is_some(), "DefaultClient.cs must be emitted");
    let handle_content = &handle_file.unwrap().content;

    assert!(
        handle_content.contains("public static DefaultClient Create("),
        "should emit public static Create factory: {handle_content}"
    );
    assert!(
        handle_content.contains("string apiKey"),
        "string param should appear as C# string in factory signature: {handle_content}"
    );
    assert!(
        handle_content.contains("NativeMethods.DefaultClientNew("),
        "factory should call NativeMethods.DefaultClientNew: {handle_content}"
    );

    // Find NativeMethods.cs — should contain the P/Invoke for _new.
    let native_methods_file = files.iter().find(|f| f.path.ends_with("NativeMethods.cs"));
    assert!(native_methods_file.is_some(), "NativeMethods.cs must be emitted");
    let native_content = &native_methods_file.unwrap().content;

    assert!(
        native_content.contains("DefaultClientNew("),
        "NativeMethods should declare DefaultClientNew P/Invoke: {native_content}"
    );
    assert!(
        native_content.contains("[MarshalAs(UnmanagedType.LPUTF8Str)] string apiKey"),
        "string param should use explicit UTF-8 marshalling in P/Invoke: {native_content}"
    );
    assert!(
        native_content.contains("IntPtr DefaultClientNew("),
        "P/Invoke should return IntPtr: {native_content}"
    );
}

#[test]
fn test_record_method_bool_param_passes_bool_directly() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TableParserConfig".to_string(),
            rust_path: "test::TableParserConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "with_table_detection".to_string(),
                params: vec![ParamDef {
                    name: "enable".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
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
                return_type: TypeRef::Named("TableParserConfig".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Enable table detection.".to_string(),
                receiver: Some(ReceiverKind::Ref),
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

    let files = backend.generate_bindings(&api, &config).unwrap();

    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TableParserConfig.cs"))
        .expect("TableParserConfig.cs should be generated");

    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .expect("NativeMethods.cs should be generated");

    // Verify P/Invoke declaration uses [MarshalAs(UnmanagedType.U1)] bool
    assert!(
        native_file
            .content
            .contains("[MarshalAs(UnmanagedType.U1)] bool enable"),
        "P/Invoke should declare bool parameter with marshaling attribute: {}",
        native_file.content
    );

    // Verify method call passes bool directly (not (enable ? 1 : 0))
    assert!(
        config_file.content.contains("enable"),
        "Bool parameter should be passed directly in method call: {}",
        config_file.content
    );

    // Verify the int conversion does NOT appear
    assert!(
        !config_file.content.contains("(enable ? 1 : 0)"),
        "Bool parameter should not be converted to int with (enable ? 1 : 0): {}",
        config_file.content
    );
}

#[test]
fn test_receiver_selfhandle_freed_on_named_param_failure() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "SomeOther".to_string(),
                rust_path: "test::SomeOther".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
                    optional: false,
                    default: None,
                    typed_default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "SomeConfig".to_string(),
                rust_path: "test::SomeConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "with_param".to_string(),
                    params: vec![ParamDef {
                        name: "other".to_string(),
                        ty: TypeRef::Named("SomeOther".to_string()),
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
                    return_type: TypeRef::Named("SomeConfig".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: Some("SomeError".to_string()),
                    doc: "Configure with other.".to_string(),
                    receiver: Some(ReceiverKind::Ref),
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
        errors: vec![ErrorDef {
            name: "SomeError".to_string(),
            rust_path: "test::SomeError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
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

    let files = backend.generate_bindings(&api, &config).unwrap();

    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SomeConfig.cs"))
        .expect("SomeConfig.cs should be generated");

    // Verify that try block starts BEFORE the named param setup
    // by checking that "try" appears before "otherHandle"
    let try_pos = config_file.content.find("try").expect("Should contain 'try' block");
    let other_handle_pos = config_file
        .content
        .find("otherHandle")
        .expect("Should contain 'otherHandle' for named param");
    assert!(
        try_pos < other_handle_pos,
        "try block must start BEFORE named param setup (otherHandle), \
         to ensure selfHandle is freed if FromJson fails: content={}",
        config_file.content
    );

    // Verify that selfHandle is freed in finally
    assert!(
        config_file.content.contains("NativeMethods.SomeConfigFree(selfHandle)"),
        "selfHandle must be freed in finally block: {}",
        config_file.content
    );
}

#[test]
fn test_record_static_factory_named_param_emits_handle_marshaling() {
    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "TextParseResult".to_string(),
                rust_path: "test::TextParseResult".to_string(),
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
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ParseResult".to_string(),
                rust_path: "test::ParseResult".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "from_text".to_string(),
                    params: vec![ParamDef {
                        name: "text_result".to_string(),
                        ty: TypeRef::Named("TextParseResult".to_string()),
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
                    return_type: TypeRef::Named("ParseResult".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create from text parse result.".to_string(),
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

    let files = backend.generate_bindings(&api, &config).unwrap();

    // Find the ParseResult file (not TextParseResult)
    let result_file = files
        .iter()
        .find(|f| {
            let fname = f.path.to_string_lossy().to_string();
            fname.contains("ParseResult.cs") && !fname.contains("TextParseResult.cs")
        })
        .expect("ParseResult.cs should be generated");

    // Should contain FromJson handle creation for the Named param
    assert!(
        result_file.content.contains("FromJson"),
        "Should create handle using FromJson: {}",
        result_file.content
    );

    // Should contain try/finally block for handle cleanup
    assert!(
        result_file.content.contains("try") && result_file.content.contains("finally"),
        "Should wrap native call in try/finally for cleanup: {}",
        result_file.content
    );

    // Should contain Free call for the Named param handle
    assert!(
        result_file.content.contains("TextParseResultFree"),
        "Should free Named param handle: {}",
        result_file.content
    );
}

/// Compile-level check: generate C# for a record type whose instance method takes a `bool`
/// parameter, write all files to a temp directory, and invoke `dotnet build` to verify the
/// generated output is free of type errors (e.g. passing `int` to a `bool` P/Invoke param).
#[test]
fn test_bool_param_record_method_compiles_with_dotnet() {
    if std::process::Command::new("dotnet").arg("--version").output().is_err() {
        eprintln!("dotnet not in PATH — skipping compile test");
        return;
    }

    let backend = CsharpBackend;
    let config = minimal_csharp_config("test");

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TableParserConfig".to_string(),
            rust_path: "test::TableParserConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "with_table_detection".to_string(),
                params: vec![ParamDef {
                    name: "enable".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
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
                return_type: TypeRef::Named("TableParserConfig".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Ref),
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
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();

    // Write generated files to a temp directory preserving their relative paths.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    for file in &files {
        let dest = tmp.path().join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).expect("failed to create dir");
        }
        std::fs::write(&dest, &file.content).expect("failed to write generated file");
    }

    // Place the .csproj alongside the generated Directory.Build.props so MSBuild
    // inherits Nullable/LangVersion/TreatWarningsAsErrors and discovers all .cs
    // files in the Test/ subdirectory automatically.
    let csproj_dir = tmp.path().join("packages/csharp");
    std::fs::create_dir_all(&csproj_dir).unwrap();
    std::fs::write(
        csproj_dir.join("Compilation.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n\
         <PropertyGroup>\n\
           <TargetFramework>net8.0</TargetFramework>\n\
           <OutputType>Library</OutputType>\n\
           <NuGetAudit>false</NuGetAudit>\n\
         </PropertyGroup>\n\
         </Project>\n",
    )
    .expect("failed to write csproj");

    let output = std::process::Command::new("dotnet")
        .args(["build", "--nologo", "-v:quiet"])
        .current_dir(&csproj_dir)
        .output()
        .expect("failed to spawn dotnet build");

    assert!(
        output.status.success(),
        "dotnet build failed — the generated C# does not compile.\n\
         This catches type mismatches such as passing int to a bool P/Invoke parameter.\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn test_trait_bridge_clear_method_uses_clear_fn_name_not_trait_name() {
    let backend = CsharpBackend;
    let mut config = minimal_csharp_config("test");

    // Add trait bridges with clear_fn configured to test the method naming
    config.trait_bridges = vec![
        alef::core::config::TraitBridgeConfig {
            trait_name: "TextBackend".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: Some("register_text_backend".to_string()),
            unregister_fn: Some("unregister_text_backend".to_string()),
            clear_fn: Some("clear_text_backends".to_string()),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            ffi_skip_methods: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        },
        alef::core::config::TraitBridgeConfig {
            trait_name: "PostProcessor".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: Some("register_post_processor".to_string()),
            unregister_fn: Some("unregister_post_processor".to_string()),
            clear_fn: Some("clear_post_processors".to_string()),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            ffi_skip_methods: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        },
    ];

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();

    // Find the wrapper class file (contains the Clear* facade methods)
    let wrapper_file = files
        .iter()
        .find(|f| {
            let path_str = f.path.to_string_lossy();
            path_str.ends_with("SampleCrateConverter.cs")
                || (path_str.ends_with(".cs") && f.content.contains("public static void Clear"))
        })
        .unwrap_or_else(|| {
            panic!(
                "No wrapper file with Clear* methods found. Generated files: {:?}",
                files
                    .iter()
                    .map(|f| f.path.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            )
        });

    let content = &wrapper_file.content;

    // Verify that the method names are ClearTextBackends and ClearPostProcessors (plural, from clear_fn)
    // NOT ClearTextBackend and ClearPostProcessor (singular, from trait name)
    assert!(
        content.contains("public static void ClearTextBackends()"),
        "Expected method ClearTextBackends (from clear_text_backends), but not found.\n\
         Check that method naming derives from clear_fn, not trait name.\nContent:\n{}",
        content
    );

    assert!(
        content.contains("public static void ClearPostProcessors()"),
        "Expected method ClearPostProcessors (from clear_post_processors), but not found.\n\
         Check that method naming derives from clear_fn, not trait name.\nContent:\n{}",
        content
    );

    // Verify that singular names are NOT present (these would be the wrong names)
    assert!(
        !content.contains("public static void ClearTextBackend()"),
        "Found incorrect method ClearTextBackend (singular). \
         Method name must derive from clear_fn (clear_text_backends → ClearTextBackends), not trait name."
    );

    assert!(
        !content.contains("public static void ClearPostProcessor()"),
        "Found incorrect method ClearPostProcessor (singular). \
         Method name must derive from clear_fn (clear_post_processors → ClearPostProcessors), not trait name."
    );
}
