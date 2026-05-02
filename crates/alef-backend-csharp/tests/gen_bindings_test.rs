use alef_backend_csharp::CsharpBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, BridgeBinding, CSharpConfig, CrateConfig, FfiConfig, TraitBridgeConfig};
use alef_core::ir::{
    ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeDef,
    TypeRef,
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
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    // Create test config
    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "kreuzberg".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: Some(FfiConfig {
            prefix: Some("kreuzberg".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: Some(CSharpConfig {
            namespace: Some("Kreuzberg".to_string()),
            package_id: None,
            target_framework: None,
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
            exclude_functions: Vec::new(),
        }),
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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
fn test_namespace_resolution() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "my-lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: Some(CSharpConfig {
            namespace: Some("MyCompany.MyLib".to_string()),
            package_id: None,
            target_framework: None,
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
            exclude_functions: Vec::new(),
        }),
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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
    };

    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,

        go: None,
        java: None,

        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,

        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

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

/// Helper: build a minimal AlefConfig with a CSharp config for crate named "test".
fn minimal_csharp_config(crate_name: &str) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: crate_name.to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: Some(FfiConfig {
            prefix: Some(crate_name.to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        go: None,
        java: None,
        csharp: Some(CSharpConfig {
            namespace: Some("Test".to_string()),
            package_id: None,
            target_framework: None,
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
            exclude_functions: Vec::new(),
        }),
        kotlin: None,
        swift: None,
        dart: None,
        gleam: None,
        zig: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    }
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            serde_rename_all: None,
        }],
        errors: vec![],
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

// ---------------------------------------------------------------------------
// Helpers for options-field bridge tests
// ---------------------------------------------------------------------------

fn make_conversion_options_type() -> TypeDef {
    TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "htm::ConversionOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "some_flag".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
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
            },
            FieldDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("HtmlVisitorHandle".to_string()),
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
            },
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
        doc: String::new(),
        cfg: None,
    }
}

fn make_html_visitor_trait() -> TypeDef {
    TypeDef {
        name: "HtmlVisitor".to_string(),
        rust_path: "htm::HtmlVisitor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "visit_text".to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(alef_core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: true,
        }],
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
    }
}

fn make_convert_function() -> FunctionDef {
    FunctionDef {
        name: "convert".to_string(),
        rust_path: "htm::convert".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "html".to_string(),
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
                name: "options".to_string(),
                ty: TypeRef::Named("ConversionOptions".to_string()),
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
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

fn make_options_field_bridge_config(crate_name: &str) -> AlefConfig {
    let mut config = minimal_csharp_config(crate_name);
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some("HtmlVisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: Some("visitor".to_string()),
    }];
    config
}

// ---------------------------------------------------------------------------
// Options-field bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_options_field_bridge_adds_visitor_property_to_options_type() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let config = make_options_field_bridge_config("htm");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let opts_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ConversionOptions.cs"))
        .expect("ConversionOptions.cs should be generated");

    let content = &opts_file.content;

    assert!(
        content.contains("[JsonIgnore]"),
        "ConversionOptions.cs must contain [JsonIgnore] for the bridge property; got:\n{content}"
    );
    assert!(
        content.contains("HtmlVisitorBridge? Visitor"),
        "ConversionOptions.cs must have a HtmlVisitorBridge? Visitor property; got:\n{content}"
    );
    assert!(
        !content.contains("[JsonPropertyName(\"visitor\")]"),
        "ConversionOptions.cs must not serialize the visitor field as JSON; got:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_emits_setter_pinvoke() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let config = make_options_field_bridge_config("htm");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let nm_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .expect("NativeMethods.cs should be generated");

    let content = &nm_file.content;

    assert!(
        content.contains("htm_options_set_visitor"),
        "NativeMethods.cs must declare the options setter entry-point; got:\n{content}"
    );
    assert!(
        content.contains("ConversionOptionsSetVisitor"),
        "NativeMethods.cs must expose ConversionOptionsSetVisitor; got:\n{content}"
    );
    assert!(
        content.contains("ConversionOptionsSetVisitor(IntPtr options, IntPtr vtable)"),
        "Setter must have (IntPtr options, IntPtr vtable) signature; got:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_wrapper_calls_setter_not_convert_with_visitor() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let config = make_options_field_bridge_config("htm");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let wrapper_file = files
        .iter()
        .find(|f| f.content.contains("public static") && f.content.contains("Convert("))
        .expect("wrapper class file should be generated");

    let content = &wrapper_file.content;

    assert!(
        content.contains("ConversionOptionsSetVisitor"),
        "Wrapper Convert must call ConversionOptionsSetVisitor; got:\n{content}"
    );
    assert!(
        !content.contains("ConvertWithVisitor"),
        "Wrapper must not expose ConvertWithVisitor in options-field mode; got:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_drops_stale_from_json_native_method() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let config = make_options_field_bridge_config("htm");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let nm_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .expect("NativeMethods.cs should be generated");

    let content = &nm_file.content;

    assert!(
        !content.contains("htm_conversion_options_from_json"),
        "NativeMethods.cs must NOT declare the deleted from_json entry-point; got:\n{content}"
    );
    assert!(
        !content.contains("htm_convert_with_visitor"),
        "NativeMethods.cs must NOT declare the deleted convert_with_visitor entry-point; got:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_wrapper_uses_update_from_json_not_from_json() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let config = make_options_field_bridge_config("htm");
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let wrapper_file = files
        .iter()
        .find(|f| f.content.contains("public static") && f.content.contains("Convert("))
        .expect("wrapper class file should be generated");

    let content = &wrapper_file.content;

    assert!(
        content.contains("ConversionOptionsFromUpdate"),
        "Wrapper Convert must use ConversionOptionsFromUpdate (not ConversionOptionsFromJson); got:\n{content}"
    );
    assert!(
        !content.contains("ConversionOptionsFromJson"),
        "Wrapper Convert must NOT call the deleted ConversionOptionsFromJson; got:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_excluded_by_language_leaves_json_field() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "htm".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_conversion_options_type(), make_html_visitor_trait()],
        functions: vec![make_convert_function()],
        enums: vec![],
        errors: vec![],
    };

    let mut config = minimal_csharp_config("htm");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some("HtmlVisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec!["csharp".to_string()],
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: Some("visitor".to_string()),
    }];

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let opts_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ConversionOptions.cs"))
        .expect("ConversionOptions.cs should be generated");

    let content = &opts_file.content;

    assert!(
        !content.contains("[JsonIgnore]"),
        "Excluded bridge must not inject [JsonIgnore]; got:\n{content}"
    );
    assert!(
        !content.contains("HtmlVisitorBridge"),
        "Excluded bridge must not inject visitor property; got:\n{content}"
    );
}
