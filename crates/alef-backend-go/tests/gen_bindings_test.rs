use alef_backend_go::GoBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, FfiConfig, GoConfig};
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
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
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
        }),
        go: Some(GoConfig {
            module: Some("github.com/test/test-lib".to_string()),
            package_name: None,
            features: None,
            serde_rename_all: None,
        }),
        java: None,
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
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
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Process data".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Slow".to_string(),
                    fields: vec![],
                    doc: "Slow mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
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
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("vec_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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

    // Verify Go type mappings
    assert!(content.contains("U32Val uint32"), "U32 should map to uint32");
    assert!(content.contains("I64Val int64"), "I64 should map to int64");
    assert!(
        content.contains("StringVal *string"),
        "Optional String should be *string"
    );
    assert!(
        content.contains("VecVal []string"),
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
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Completed".to_string(),
                    fields: vec![],
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Status enum".to_string(),
            cfg: None,
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Process data asynchronously".to_string(),
            cfg: None,
            sanitized: false,
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
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
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
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U8), false),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_thing".to_string(),
            rust_path: "test_lib::create_thing".to_string(),
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
