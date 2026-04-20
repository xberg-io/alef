use alef_backend_pyo3::Pyo3Backend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, PythonConfig, StubsConfig};
use alef_core::ir::*;
use std::path::PathBuf;

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

fn make_config_with_stubs() -> AlefConfig {
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
        python: Some(PythonConfig {
            module_name: Some("_test_lib".to_string()),
            pip_name: None,
            async_runtime: None,
            stubs: Some(StubsConfig {
                output: PathBuf::from("packages/python/src/"),
            }),
            features: None,
            serde_rename_all: None,
            capsule_types: Default::default(),
            release_gil: false,
        }),
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        go: None,
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
fn test_basic_stubs() {
    let backend = Pyo3Backend;

    // Create test API surface with 1 TypeDef (2 fields), 1 FunctionDef, 1 EnumDef (2 variants)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, false),
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
            doc: "Test configuration".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
            is_async: false,
            error_type: None,
            doc: "Process input".to_string(),
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
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
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

    let config = make_config_with_stubs();

    // Generate type stubs
    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Failed to generate type stubs");
    let files = result.unwrap();

    // Should generate 1 file: _test_lib.pyi
    assert_eq!(files.len(), 1, "Expected 1 generated stub file");

    let stub_file = &files[0];
    assert!(stub_file.path.to_string_lossy().ends_with(".pyi"), "Expected .pyi file");

    let content = &stub_file.content;

    // Assert header
    assert!(
        content.contains("# This file is auto-generated by alef. DO NOT EDIT."),
        "Should contain generated header"
    );
    assert!(content.contains("from typing import"), "Should import typing module");

    // Assert type stub
    assert!(content.contains("class Config:"), "Should define Config class stub");
    assert!(
        content.contains("timeout: int"),
        "Should have timeout field with int type"
    );
    assert!(content.contains("name: str"), "Should have name field with str type");
    assert!(content.contains("def __init__(self"), "Should have __init__ signature");

    // Assert function stub
    assert!(
        content.contains("def process(input: str) -> str:"),
        "Should have process function stub with type annotations"
    );

    // Assert enum stub
    assert!(content.contains("class Mode:"), "Should define Mode enum class stub");
    assert!(
        content.contains("Fast: Mode = ..."),
        "Should have Fast variant typed as Mode"
    );
    assert!(
        content.contains("Accurate: Mode = ..."),
        "Should have Accurate variant typed as Mode"
    );
    assert!(
        content.contains("def __init__(self, value: int | str) -> None:"),
        "Enum should have __init__ with int | str type"
    );
}

#[test]
fn test_optional_field_stubs() {
    let backend = Pyo3Backend;

    // TypeDef with Optional fields
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Request".to_string(),
            rust_path: "test_lib::Request".to_string(),
            fields: vec![
                make_field("url", TypeRef::String, false),
                make_field("headers", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field(
                    "timeout_ms",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                    true,
                ),
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
            doc: "HTTP request".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert class definition
    assert!(content.contains("class Request:"), "Should define Request class stub");

    // Assert required field
    assert!(
        content.contains("url: str"),
        "Required field should have type without None"
    );

    // Assert optional field with str type - should have `| None` pattern
    assert!(
        (content.contains("headers: str | None") || content.contains("headers: Optional[str]")),
        "Optional str field should have Optional[str] or str | None"
    );

    // Assert optional field with u64 type
    assert!(
        (content.contains("timeout_ms: int | None") || content.contains("timeout_ms: Optional[int]")),
        "Optional int field should have Optional[int] or int | None"
    );

    // Assert __init__ signature with optional parameters
    assert!(content.contains("def __init__("), "Should have __init__ method");
    // Optional parameters should have default None
    assert!(
        content.contains("= None"),
        "Optional parameters should have = None default"
    );
}

#[test]
fn test_enum_stubs() {
    let backend = Pyo3Backend;

    // EnumDef with multiple variants
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
                    name: "Complete".to_string(),
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

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert enum class definition
    assert!(content.contains("class Status:"), "Should define Status enum class");

    // Assert enum variants typed as the enum class itself
    assert!(
        content.contains("Pending: Status = ..."),
        "Should have Pending variant typed as Status"
    );
    assert!(
        content.contains("Active: Status = ..."),
        "Should have Active variant typed as Status"
    );
    assert!(
        content.contains("Complete: Status = ..."),
        "Should have Complete variant typed as Status"
    );

    // Assert enum __init__ signature
    assert!(
        content.contains("def __init__(self, value: int | str) -> None: ..."),
        "Enum should have __init__(self, value: int | str) -> None"
    );
}

#[test]
fn test_stubs_with_no_stubs_config() {
    let backend = Pyo3Backend;

    // API surface
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    // Config WITHOUT stubs configuration
    let mut config = make_config_with_stubs();
    config.python.as_mut().unwrap().stubs = None;

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    // When stubs config is None, should return empty vec
    assert_eq!(files.len(), 0, "Should return empty when stubs config is None");
}

#[test]
fn test_type_stubs_with_vec_fields() {
    let backend = Pyo3Backend;

    // TypeDef with Vec fields
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Collection".to_string(),
            rust_path: "test_lib::Collection".to_string(),
            fields: vec![
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field(
                    "counts",
                    TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                    false,
                ),
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
            doc: "Collection type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert class definition
    assert!(content.contains("class Collection:"), "Should define Collection class");

    // Assert Vec field types - should be List[type]
    assert!(
        content.contains("items: list[str]") || content.contains("items: List[str]"),
        "Vec<String> should map to list[str] or List[str]"
    );

    assert!(
        content.contains("counts: list[int]") || content.contains("counts: List[int]"),
        "Vec<i32> should map to list[int] or List[int]"
    );
}

#[test]
fn test_function_stubs_with_multiple_params() {
    let backend = Pyo3Backend;

    // FunctionDef with multiple params (required and optional)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_request".to_string(),
            rust_path: "test_lib::create_request".to_string(),
            params: vec![
                ParamDef {
                    name: "url".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                },
                ParamDef {
                    name: "method".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                },
                ParamDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Create request".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert function stub exists
    assert!(
        content.contains("def create_request("),
        "Should define create_request function"
    );

    // Assert required parameter (url) comes first without default
    assert!(content.contains("url: str"), "Should have url parameter with str type");

    // Assert optional parameters have default None
    assert!(
        content.contains("method: str | None = None") || content.contains("method: Optional[str] = None"),
        "Optional method parameter should have = None"
    );

    assert!(
        content.contains("timeout: int | None = None") || content.contains("timeout: Optional[int] = None"),
        "Optional timeout parameter should have = None"
    );

    // Assert return type
    assert!(content.contains("-> str:"), "Function should return str");
}

#[test]
fn test_opaque_type_stubs() {
    let backend = Pyo3Backend;

    // Opaque TypeDef with methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handler".to_string(),
            rust_path: "test_lib::Handler".to_string(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "handle".to_string(),
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
                is_static: false,
                error_type: None,
                doc: "Handle data".to_string(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
            }],
            is_opaque: true,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handler".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert opaque class definition
    assert!(content.contains("class Handler:"), "Should define Handler class stub");

    // Assert method stub
    assert!(
        content.contains("def handle(self, data: str) -> str:"),
        "Should have handle method stub"
    );

    // Opaque types should not have explicit field types (no fields)
    assert!(
        !content.contains("Handler: ") || !content.contains("fields"),
        "Opaque type should not list fields"
    );
}

#[test]
fn test_stubs_generated_header_flag() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let file = &files[0];
    // Check that generated_header flag is set to true
    assert!(
        file.generated_header,
        "Stub file should have generated_header flag set to true"
    );

    // Content should also contain the header comment
    assert!(
        file.content
            .contains("# This file is auto-generated by alef. DO NOT EDIT."),
        "Content should have generation marker"
    );
}

#[test]
fn test_python_keyword_escaping_function_name() {
    let backend = Pyo3Backend;

    // Function with name that is a Python keyword
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "pass".to_string(), // Python keyword
            rust_path: "test_lib::pass".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Pass function".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Python keyword function name 'pass' should be escaped to 'pass_'
    assert!(
        content.contains("def pass_() -> str:"),
        "Python keyword function name 'pass' should be escaped to 'pass_'"
    );
}

#[test]
fn test_static_method_stubs() {
    let backend = Pyo3Backend;

    // Type with static method
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Utils".to_string(),
            rust_path: "test_lib::Utils".to_string(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "parse".to_string(),
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
                is_async: false,
                is_static: true,
                error_type: None,
                doc: "Parse input".to_string(),
                receiver: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
            }],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Utilities".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert class definition
    assert!(content.contains("class Utils:"), "Should define Utils class");

    // Assert static method with @staticmethod decorator
    assert!(
        content.contains("@staticmethod"),
        "Static method should have @staticmethod decorator"
    );

    assert!(
        content.contains("def parse(input: str) -> str:"),
        "Static method should not have 'self' parameter"
    );
}

#[test]
fn test_multiple_types_and_functions() {
    let backend = Pyo3Backend;

    // Multiple types, functions, and enums
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "User".to_string(),
                rust_path: "test_lib::User".to_string(),
                fields: vec![
                    make_field("id", TypeRef::Primitive(PrimitiveType::U64), false),
                    make_field("name", TypeRef::String, false),
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
            },
            TypeDef {
                name: "Post".to_string(),
                rust_path: "test_lib::Post".to_string(),
                fields: vec![
                    make_field("title", TypeRef::String, false),
                    make_field("content", TypeRef::Optional(Box::new(TypeRef::String)), true),
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
            },
        ],
        functions: vec![
            FunctionDef {
                name: "get_user".to_string(),
                rust_path: "test_lib::get_user".to_string(),
                params: vec![ParamDef {
                    name: "id".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                }],
                return_type: TypeRef::Named("User".to_string()),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
            FunctionDef {
                name: "create_post".to_string(),
                rust_path: "test_lib::create_post".to_string(),
                params: vec![
                    ParamDef {
                        name: "title".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                    },
                    ParamDef {
                        name: "user_id".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U64),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                    },
                ],
                return_type: TypeRef::Named("Post".to_string()),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
        ],
        enums: vec![EnumDef {
            name: "SortOrder".to_string(),
            rust_path: "test_lib::SortOrder".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Asc".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Desc".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert both types are defined
    assert!(content.contains("class User:"), "Should define User class");
    assert!(content.contains("class Post:"), "Should define Post class");

    // Assert both functions are defined
    assert!(
        content.contains("def get_user(id: int) -> User:"),
        "Should define get_user function"
    );
    assert!(
        content.contains("def create_post(title: str, user_id: int) -> Post:"),
        "Should define create_post function"
    );

    // Assert enum is defined
    assert!(content.contains("class SortOrder:"), "Should define SortOrder enum");
    assert!(
        content.contains("Asc: SortOrder = ..."),
        "Should have Asc variant typed as SortOrder"
    );
    assert!(
        content.contains("Desc: SortOrder = ..."),
        "Should have Desc variant typed as SortOrder"
    );
}

#[test]
fn test_builtin_shadowing_params_get_noqa_comment() {
    let backend = Pyo3Backend;

    // A type whose fields use names that shadow Python builtins: `id` and `format`.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Item".to_string(),
            rust_path: "test_lib::Item".to_string(),
            fields: vec![
                make_field("id", TypeRef::Primitive(PrimitiveType::U64), false),
                make_field("format", TypeRef::String, false),
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

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let content = result.unwrap().into_iter().next().unwrap().content;

    // The __init__ must be multi-line and carry `# noqa: A002` on builtin-shadowing params.
    assert!(
        content.contains("# noqa: A002"),
        "Builtin-shadowing params must have `# noqa: A002` comment"
    );
    assert!(content.contains("id: int"), "id field should be present with int type");
    assert!(
        content.contains("format: str"),
        "format field should be present with str type"
    );
}
