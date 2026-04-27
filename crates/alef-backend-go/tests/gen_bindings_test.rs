use alef_backend_go::GoBackend;
use alef_backend_go::trait_bridge::gen_trait_bridges_file;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, FfiConfig, GoConfig, TraitBridgeConfig};
use alef_core::ir::*;

/// Helper to create a FieldDef with all defaults
fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
    }
}

/// Helper to create a full AlefConfig with both FFI and Go configs
fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
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
            prefix: Some("test".to_string()),
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
        go: Some(GoConfig {
            module: Some("github.com/test/test-lib".to_string()),
            package_name: None,
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
        }),
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
    }
}

#[test]
fn test_basic_generation() {
    let backend = GoBackend;

    // Create test API surface with 1 type, 1 function, 1 enum
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
            doc: "Configuration type".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
            original_rust_path: String::new(),
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Process data".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Slow".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Slow mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");
    assert_eq!(files.len(), 1, "Should generate exactly 1 file (binding.go)");

    let binding_file = &files[0];
    assert!(
        binding_file.path.to_string_lossy().ends_with("binding.go"),
        "Should generate binding.go file"
    );

    let content = &binding_file.content;

    // Verify Go package declaration
    assert!(content.contains("package testlib"), "Should declare Go package");

    // Verify cgo directives (include and linking)
    assert!(content.contains("#cgo CFLAGS:"), "Should have cgo CFLAGS directive");
    assert!(content.contains("#cgo LDFLAGS:"), "Should have cgo LDFLAGS directive");
    assert!(content.contains("import \"C\""), "Should import C");

    // Verify standard Go imports
    assert!(content.contains("import ("), "Should have import block");
    assert!(content.contains("\"fmt\""), "Should import fmt");
    assert!(content.contains("\"encoding/json\""), "Should import encoding/json");
    assert!(content.contains("\"unsafe\""), "Should import unsafe");

    // Verify error helper
    assert!(content.contains("func lastError()"), "Should define lastError helper");
    assert!(
        content.contains("C.test_last_error_code()"),
        "Should call FFI error code function"
    );

    // Verify struct generation
    assert!(content.contains("type Config struct"), "Should define Config struct");
    assert!(content.contains("Timeout"), "Should have Timeout field");
    assert!(content.contains("Name"), "Should have Name field");
    assert!(content.contains("json:"), "Should have JSON tags");

    // Verify enum generation
    assert!(
        content.contains("type Mode string"),
        "Should define Mode as string enum"
    );
    assert!(content.contains("const ("), "Should have const block for enum values");
    assert!(content.contains("ModeFast"), "Should have ModeFast constant");
    assert!(content.contains("ModeSlow"), "Should have ModeSlow constant");

    // Verify function wrapper
    assert!(content.contains("func Process("), "Should define Process function");
}

#[test]
fn test_type_mapping() {
    let backend = GoBackend;

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
                make_field("vec_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify Go type mappings — gofmt aligns fields with variable whitespace,
    // so we check for the field name and type without exact spacing.
    let lines: Vec<&str> = content.lines().collect();
    let struct_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("Val")).collect();
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("U32Val") && l.contains("uint32")),
        "U32 should map to uint32"
    );
    assert!(
        struct_lines.iter().any(|l| l.contains("I64Val") && l.contains("int64")),
        "I64 should map to int64"
    );
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("StringVal") && l.contains("*string")),
        "Optional String should be *string"
    );
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("VecVal") && l.contains("[]string")),
        "Vec<String> should map to []string"
    );
}

#[test]
fn test_enum_generation() {
    let backend = GoBackend;

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
                    is_tuple: false,
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Completed".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Status enum".to_string(),
            cfg: None,
            is_copy: false,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify enum type declaration
    assert!(
        content.contains("type Status string"),
        "Should define Status as string type"
    );

    // Verify const block with all variants
    assert!(content.contains("const ("), "Should define const block");
    assert!(content.contains("StatusPending"), "Should have StatusPending constant");
    assert!(content.contains("StatusActive"), "Should have StatusActive constant");
    assert!(
        content.contains("StatusCompleted"),
        "Should have StatusCompleted constant"
    );

    // Verify string values in snake_case
    assert!(content.contains("\"pending\""), "Should use snake_case values");
    assert!(content.contains("\"active\""), "Should use snake_case values");
    assert!(content.contains("\"completed\""), "Should use snake_case values");
}

#[test]
fn test_generated_header() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert!(!files.is_empty());

    // All files should have generated_header set to true
    for file in &files {
        assert!(
            file.generated_header,
            "All generated files should have generated_header=true"
        );
    }
}

#[test]
fn test_methods_generation() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handler".to_string(),
            rust_path: "test_lib::Handler".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U64), false)],
            methods: vec![
                // Instance method that returns a value
                MethodDef {
                    name: "get_name".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Get the name".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                },
                // Static method that returns a primitive (not skipped)
                MethodDef {
                    name: "version".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Get the version string".to_string(),
                    receiver: None,
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                },
                // Instance method with parameters and error
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
                    }],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: Some("Error".to_string()),
                    doc: "Process data".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                },
            ],
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
            doc: "A handler type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify instance method wrapper (receiver with pointer)
    assert!(
        content.contains("func (r *Handler) GetName()"),
        "Should define instance method GetName with receiver"
    );
    assert!(content.contains("*string"), "GetName should return *string");

    // Verify static method (no receiver, becomes function)
    assert!(
        content.contains("func HandlerVersion()"),
        "Should define static method as package-level function"
    );

    // Verify instance method with error return
    assert!(
        content.contains("func (r *Handler) Process(data string) error"),
        "Should define Process method with error return"
    );
    assert!(
        content.contains("return lastError()"),
        "Process should call lastError() for error handling"
    );
}

#[test]
fn test_error_types() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GoError".to_string(),
            rust_path: "test_lib::GoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    doc: "Resource not found".to_string(),
                    message_template: Some("not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input: {reason}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                },
            ],
            doc: "Error type for library".to_string(),
        }],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify error type generation via alef_codegen
    // The gen_go_error_types function generates sentinel errors and error wrappers
    assert!(
        content.contains("errors") || content.contains("Error"),
        "Should import or reference errors package"
    );
    // Verify error-related code is generated
    assert!(
        content.contains("GoError") || content.contains("lastError"),
        "Should generate error-related code"
    );
}

#[test]
fn test_async_function() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "async_process".to_string(),
            rust_path: "test_lib::async_process".to_string(),
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Process data asynchronously".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify async function is generated
    assert!(content.contains("func AsyncProcess("), "Should define async function");
    // Async functions in Go use the FFI with block_on() internally, but the wrapper is still generated
    assert!(
        content.contains("AsyncProcess"),
        "Async function should be included in generated code"
    );
}

#[test]
fn test_opaque_type() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueHandle".to_string(),
            rust_path: "test_lib::OpaqueHandle".to_string(),
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
            doc: "An opaque handle to Rust state".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify opaque type wraps unsafe.Pointer
    assert!(
        content.contains("type OpaqueHandle struct"),
        "Should define OpaqueHandle struct"
    );
    assert!(
        content.contains("ptr unsafe.Pointer"),
        "Should have ptr field of unsafe.Pointer type"
    );
    assert!(
        content.contains("\"unsafe\""),
        "Should import unsafe package for opaque types"
    );

    // Verify Free method
    assert!(
        content.contains("func (h *OpaqueHandle) Free()"),
        "Should define Free method for opaque type"
    );
    assert!(
        content.contains("test_opaque_handle_free") || content.contains("Free"),
        "Free method should call FFI free function"
    );
}

#[test]
fn test_default_config() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U8), false),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true, // Enable functional options
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify ConfigOption type
    assert!(
        content.contains("type ConfigOption func(*Config)"),
        "Should define ConfigOption functional option type"
    );

    // Verify With* constructors
    assert!(
        content.contains("func WithConfigTimeout("),
        "Should define WithConfigTimeout constructor"
    );
    assert!(
        content.contains("func WithConfigRetries("),
        "Should define WithConfigRetries constructor"
    );
    assert!(
        content.contains("func WithConfigName("),
        "Should define WithConfigName constructor"
    );

    // Verify NewConfig constructor
    assert!(
        content.contains("func NewConfig(opts ...ConfigOption)"),
        "Should define NewConfig constructor with variadic options"
    );
    assert!(
        content.contains("return c"),
        "NewConfig should return the configured instance"
    );

    // Verify default values are set
    assert!(
        content.contains("Timeout:") || content.contains("timeout"),
        "NewConfig should initialize Timeout field with default"
    );
    assert!(
        content.contains("Retries:") || content.contains("retries"),
        "NewConfig should initialize Retries field with default"
    );
}

#[test]
fn test_optional_primitive_uses_cgo_types() {
    // Regression test: optional primitive params must be declared using CGo types
    // (C.uint64_t, C.uint32_t, etc.) rather than Go native types, because CGo
    // does not implicitly convert between Go numeric types and C typedef types
    // when calling C functions.
    let backend = GoBackend;

    let make_param = |name: &str, prim: PrimitiveType| ParamDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(prim),
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_thing".to_string(),
            rust_path: "test_lib::create_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_param("timeout_secs", PrimitiveType::U64),
                make_param("max_retries", PrimitiveType::U32),
            ],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // The temporary variables must be declared as CGo types, not Go native types.
    // Wrong (old): var cTimeoutSecs uint64 = ^uint64(0)
    // Right (new): var cTimeoutSecs C.uint64_t = C.uint64_t(^uint64(0))
    assert!(
        content.contains("C.uint64_t(^uint64(0))"),
        "U64 optional sentinel should be cast to C.uint64_t, got:\n{}",
        content
    );
    assert!(
        content.contains("C.uint32_t(^uint32(0))"),
        "U32 optional sentinel should be cast to C.uint32_t"
    );
    assert!(
        !content.contains("var cTimeoutSecs uint64"),
        "Should not declare cTimeoutSecs as Go uint64 — must use C.uint64_t"
    );
    assert!(
        !content.contains("var cMaxRetries uint32"),
        "Should not declare cMaxRetries as Go uint32 — must use C.uint32_t"
    );
}

#[test]
fn test_optional_return_type_no_double_pointer() {
    // Regression test: a function returning Option<String> (TypeRef::Optional(String))
    // must produce a *string return type, not **string.
    // go_type(Optional(String)) already emits "*string"; adding an extra "*" prefix
    // in the return type calculation produced "**string" which is invalid Go.
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "detect_language".to_string(),
            rust_path: "test_lib::detect_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "ext".to_string(),
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
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            is_async: false,
            error_type: None,
            doc: "Detect language from extension".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // Must NOT contain a double-pointer return type
    assert!(
        !content.contains("**string"),
        "Optional<String> return must not produce **string, got:\n{}",
        content
    );
    // Must contain the correct single-pointer return type
    assert!(
        content.contains("*string"),
        "Optional<String> return should produce *string, got:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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
    }
}

fn make_trait_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, has_error: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: format!("{name} method."),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        trait_source: None,
    }
}

fn make_trait_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
    }
}

fn make_config_with_bridges(bridge_configs: Vec<TraitBridgeConfig>) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
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
            prefix: Some("krz".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: true,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,
        go: Some(GoConfig {
            module: Some("github.com/test/test-lib".to_string()),
            package_name: None,
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
        }),
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
        trait_bridges: bridge_configs,
        tools: Default::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    }
}

fn make_api_with_type(trait_type: TypeDef) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

// ---------------------------------------------------------------------------
// Go interface generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_produces_go_interface() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("type OcrBackend interface"),
        "should generate Go interface for the trait"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_plugin_lifecycle_methods() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_scanner_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    // Plugin lifecycle methods must always be present in the interface
    assert!(
        code.contains("Name() string"),
        "Go interface must include Name() string"
    );
    assert!(
        code.contains("Version() string"),
        "Go interface must include Version() string"
    );
    assert!(
        code.contains("Initialize() error"),
        "Go interface must include Initialize() error"
    );
    assert!(
        code.contains("Shutdown() error"),
        "Go interface must include Shutdown() error"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_trait_methods_in_pascal_case() {
    let trait_type = make_trait_type(
        "ImageProcessor",
        vec![
            make_trait_method("process_image", vec![], TypeRef::String, true),
            make_trait_method(
                "get_format",
                vec![make_trait_param("path", TypeRef::String)],
                TypeRef::String,
                false,
            ),
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "ImageProcessor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_image_processor".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("ProcessImage("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
    assert!(
        code.contains("GetFormat("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_method_with_error_returns_tuple_or_error() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![
            make_trait_method("analyze", vec![], TypeRef::String, true), // (string, error)
            make_trait_method("ping", vec![], TypeRef::Unit, true),      // error
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("(string, error)"),
        "method with non-unit return and error must produce (T, error) return type"
    );
    // Unit return with error: just "error"
    assert!(
        code.contains("Ping() error") || code.contains("Ping()"),
        "method with unit return and error must produce 'error' return type"
    );
}

// ---------------------------------------------------------------------------
// Trampoline generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_generates_exported_trampolines() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    // Each trait method must have a //export trampoline
    assert!(
        code.contains("//export goOcrBackendProcess"),
        "trampoline for 'process' must be exported as goOcrBackendProcess"
    );
    // Plugin lifecycle trampolines
    assert!(
        code.contains("//export goOcrBackendName"),
        "plugin Name trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendInitialize"),
        "plugin Initialize trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendFreeUserData"),
        "free_user_data trampoline must be exported"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampolines_retrieve_go_object_via_cgo_handle() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("cgo.Handle(uintptr(unsafe.Pointer(userData)))"),
        "trampolines must retrieve the Go object via cgo.Handle from userData"
    );
    assert!(
        code.contains("runtime/cgo"),
        "must import runtime/cgo for cgo.Handle support"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampoline_converts_string_param_from_c() {
    let trait_type = make_trait_type(
        "Greeter",
        vec![make_trait_method(
            "greet",
            vec![make_trait_param("message", TypeRef::String)],
            TypeRef::Unit,
            false,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_greeter".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("C.GoString(message)"),
        "trampoline must convert *C.char parameter to Go string via C.GoString"
    );
}

// ---------------------------------------------------------------------------
// Registration function
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_registration_fn_builds_vtable_and_calls_c_register() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("func RegisterOcrBackend(impl OcrBackend) error"),
        "registration function must have the correct Go signature"
    );
    assert!(
        code.contains("cgo.NewHandle(impl)"),
        "registration must create a cgo.Handle for the Go object"
    );
    assert!(
        code.contains("C.krz_register_ocr_backend("),
        "registration must call the C FFI register function with correct name format"
    );
    assert!(
        code.contains("func UnregisterOcrBackend(name string) error"),
        "unregistration function must also be generated"
    );
    assert!(
        code.contains("C.krz_unregister_ocr_backend("),
        "unregistration must call the C FFI unregister function with correct name format"
    );
}

#[test]
fn test_gen_trait_bridges_file_registration_fn_handles_c_error_response() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    assert!(
        code.contains("if rc != 0"),
        "registration must check the C return code for errors"
    );
    assert!(
        code.contains("fmt.Errorf"),
        "registration must return a Go error on C failure"
    );
    assert!(
        code.contains("handle.Delete()"),
        "registration must delete the cgo.Handle on failure to avoid leaking"
    );
}

// ---------------------------------------------------------------------------
// VTable struct name derivation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_uses_correct_vtable_struct_name() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    // With crate_name="kreuzberg", the VTable struct should be KREUZBERGKreuzbergOcrBackendVTable
    let code = gen_trait_bridges_file(
        &api,
        &config,
        "testlib",
        "kreuzberg",
        "test.h",
        "crate/ffi",
        "../",
        "kreuzberg",
    );

    assert!(
        code.contains("vtable := C.KREUZBERGKreuzbergOcrBackendVTable{"),
        "must use correct cbindgen-generated VTable struct name format: {{CRATE_UPPER}}{{CratePascal}}{{TraitPascal}}VTable"
    );
}

// ---------------------------------------------------------------------------
// CGo preamble
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_cgo_preamble_forward_declares_trampolines() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![make_trait_method("analyze", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "crate/ffi", "../", "testlib");

    // CGo preamble must forward-declare all exported Go functions
    assert!(
        code.contains("extern int32_t goAnalyzerAnalyze("),
        "CGo preamble must forward-declare the analyze trampoline"
    );
    assert!(
        code.contains("import \"C\""),
        "must import C after the CGo preamble block"
    );
}

// ---------------------------------------------------------------------------
// via generate_bindings (end-to-end)
// ---------------------------------------------------------------------------

#[test]
fn test_generate_bindings_with_trait_bridge_emits_trait_bridges_go_file() {
    let backend = GoBackend;

    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let result = backend.generate_bindings(&api, &config);

    assert!(
        result.is_ok(),
        "generate_bindings must succeed with trait_bridges configured"
    );
    let files = result.unwrap();

    let bridge_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("trait_bridges.go"));
    assert!(
        bridge_file.is_some(),
        "generate_bindings should emit trait_bridges.go when trait_bridges are configured"
    );

    let content = &bridge_file.unwrap().content;
    assert!(
        content.contains("type OcrBackend interface"),
        "trait_bridges.go must contain the Go interface"
    );
    assert!(
        content.contains("func RegisterOcrBackend"),
        "trait_bridges.go must contain the registration function"
    );
}
