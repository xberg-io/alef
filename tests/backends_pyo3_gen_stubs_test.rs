use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

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
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_config_with_stubs() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/src/"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
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
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, false),
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
            is_async: false,
            error_type: None,
            doc: "Process input".to_string(),
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
        content.contains("# This file is auto-generated by alef"),
        "Should contain generated header"
    );
    // This surface uses only primitive types and a simple enum — no typing imports needed.
    assert!(
        !content.contains("from typing import"),
        "Should not import typing for a simple surface"
    );

    // Assert type stub
    assert!(content.contains("class Config:"), "Should define Config class stub");
    assert!(
        content.contains("timeout: int"),
        "Should have timeout field with int type"
    );
    assert!(content.contains("name: str"), "Should have name field with str type");
    assert!(content.contains("def __init__(self"), "Should have __init__ signature");

    // Assert function stub — `input` shadows a builtin, so signature is multi-line with noqa
    assert!(
        content.contains("def process(\n    input: str,  # noqa: A002\n) -> str:"),
        "Should have process function stub with multi-line signature (input is a builtin)"
    );

    // Assert enum stub — variants are emitted as SHOUTY_SNAKE_CASE to match pyo3 runtime
    assert!(content.contains("class Mode:"), "Should define Mode enum class stub");
    assert!(
        content.contains("FAST: Mode = ..."),
        "Should have FAST variant typed as Mode (matches pyo3 runtime SHOUTY_SNAKE_CASE)"
    );
    assert!(
        content.contains("ACCURATE: Mode = ..."),
        "Should have ACCURATE variant typed as Mode (matches pyo3 runtime SHOUTY_SNAKE_CASE)"
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
            original_rust_path: String::new(),
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
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "HTTP request".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
                    doc: "Completed status".to_string(),
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
            doc: "Status enum".to_string(),
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

    let config = make_config_with_stubs();

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert enum class definition
    assert!(content.contains("class Status:"), "Should define Status enum class");

    // Assert enum variants typed as the enum class itself — SHOUTY_SNAKE_CASE (matches pyo3 runtime)
    assert!(
        content.contains("PENDING: Status = ..."),
        "Should have PENDING variant typed as Status (SHOUTY_SNAKE_CASE matches pyo3 runtime)"
    );
    assert!(
        content.contains("ACTIVE: Status = ..."),
        "Should have ACTIVE variant typed as Status (SHOUTY_SNAKE_CASE matches pyo3 runtime)"
    );
    assert!(
        content.contains("COMPLETE: Status = ..."),
        "Should have COMPLETE variant typed as Status (SHOUTY_SNAKE_CASE matches pyo3 runtime)"
    );

    // Assert enum __init__ signature
    assert!(
        content.contains("def __init__(self, value: int | str) -> None: ..."),
        "Enum should have __init__(self, value: int | str) -> None"
    );
}

#[test]
fn test_exception_stubs() {
    let backend = Pyo3Backend;

    // The native module defines exceptions via create_exception! and exceptions.py re-exports
    // them, so the _native stub must declare the exception classes (base under Exception, each
    // variant under the base) or mypy reports `_native` "has no attribute" (tslp issue #147).
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "LibError".to_string(),
            rust_path: "test_lib::LibError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "Download".to_string(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Download error.".to_string(),
                },
                ErrorVariant {
                    name: "ParseFailed".to_string(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Parse failed.".to_string(),
                },
            ],
            doc: "Library errors.".to_string(),
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

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();
    let content = &files[0].content;

    // Base derives from Exception; variants (Error-suffixed, N818) derive from the base.
    assert!(
        content.contains("class LibError(Exception): ..."),
        "base error must derive from Exception:\n{content}"
    );
    assert!(
        content.contains("class DownloadError(LibError): ..."),
        "variant must derive from the base error:\n{content}"
    );
    assert!(
        content.contains("class ParseFailedError(LibError): ..."),
        "variant must derive from the base error:\n{content}"
    );
    // The base must be declared before the variants reference it as a base class.
    assert!(
        content.find("class LibError(Exception)").unwrap() < content.find("class DownloadError(LibError)").unwrap(),
        "base must be declared before variants:\n{content}"
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
            original_rust_path: String::new(),
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
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Collection type".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
            original_rust_path: String::new(),
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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
            error_type: None,
            doc: "Create request".to_string(),
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
            original_rust_path: String::new(),
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
                error_type: None,
                doc: "Handle data".to_string(),
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
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handler".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
        file.content.contains("# This file is auto-generated by alef"),
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
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Pass function".to_string(),
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
            original_rust_path: String::new(),
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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
            doc: "Utilities".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

    // `input` shadows the Python builtin, so the stub is forced to multi-line with noqa.
    assert!(content.contains("def parse("), "Static method should be defined");
    assert!(
        !content.contains("def parse(self"),
        "Static method should not have 'self' parameter"
    );
    assert!(
        content.contains("input: str"),
        "Static method should have input parameter"
    );
    assert!(
        content.contains("# noqa: A002"),
        "input param shadows builtin, must have noqa comment"
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
                original_rust_path: String::new(),
                fields: vec![
                    make_field("id", TypeRef::Primitive(PrimitiveType::U64), false),
                    make_field("name", TypeRef::String, false),
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
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
                name: "Post".to_string(),
                rust_path: "test_lib::Post".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("title", TypeRef::String, false),
                    make_field("content", TypeRef::Optional(Box::new(TypeRef::String)), true),
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
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![
            FunctionDef {
                name: "get_user".to_string(),
                rust_path: "test_lib::get_user".to_string(),
                original_rust_path: String::new(),
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
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Named("User".to_string()),
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
            },
            FunctionDef {
                name: "create_post".to_string(),
                rust_path: "test_lib::create_post".to_string(),
                original_rust_path: String::new(),
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
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: alef::core::ir::CoreWrapper::None,
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
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: alef::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("Post".to_string()),
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
            },
        ],
        enums: vec![EnumDef {
            name: "SortOrder".to_string(),
            rust_path: "test_lib::SortOrder".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Asc".to_string(),
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
                    name: "Desc".to_string(),
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
            doc: String::new(),
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
    // `id` shadows a builtin, so signature is multi-line with noqa
    assert!(
        content.contains("def get_user(\n    id: int,  # noqa: A002\n) -> User:"),
        "Should define get_user function (multi-line, id is a builtin)"
    );
    assert!(
        content.contains("def create_post(title: str, user_id: int) -> Post:"),
        "Should define create_post function"
    );

    // Assert enum is defined
    assert!(content.contains("class SortOrder:"), "Should define SortOrder enum");
    assert!(
        content.contains("ASC: SortOrder = ..."),
        "Should have ASC variant typed as SortOrder (SHOUTY_SNAKE_CASE matches pyo3 runtime)"
    );
    assert!(
        content.contains("DESC: SortOrder = ..."),
        "Should have DESC variant typed as SortOrder (SHOUTY_SNAKE_CASE matches pyo3 runtime)"
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
            original_rust_path: String::new(),
            fields: vec![
                make_field("id", TypeRef::Primitive(PrimitiveType::U64), false),
                make_field("format", TypeRef::String, false),
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
            has_private_fields: false,
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

#[test]
fn test_async_function_stub_uses_async_def() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch".to_string(),
            rust_path: "test_lib::fetch".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "url".to_string(),
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

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config).unwrap();
    let content = result.into_iter().next().unwrap().content;

    assert!(
        content.contains("async def fetch(url: str) -> str: ..."),
        "async function stub must use `async def`, got: {}",
        content
    );
}

#[test]
fn test_async_method_stub_uses_async_def() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "test_lib::Client".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![alef::core::ir::MethodDef {
                name: "send".to_string(),
                params: vec![ParamDef {
                    name: "msg".to_string(),
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
                doc: String::new(),
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
            doc: String::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config).unwrap();
    let content = result.into_iter().next().unwrap().content;

    assert!(
        content.contains("async def send(self, msg: str) -> str: ..."),
        "async method stub must use `async def`, got: {}",
        content
    );
}

// ==============================================================================
// Regression tests: UPPER_SNAKE_CASE pyclass enum variants (iter35 wave-1 W2)
// ==============================================================================

fn make_batch_status_enum_def() -> EnumDef {
    EnumDef {
        name: "BatchStatus".to_string(),
        rust_path: "test_lib::BatchStatus".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Validating".to_string(),
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
            },
            EnumVariant {
                name: "InProgress".to_string(),
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
                name: "Complete".to_string(),
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
        methods: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// `.pyi` stub emits UPPER_SNAKE_CASE attribute names (not PascalCase) for pyclass enum variants.
#[test]
fn test_pyi_stub_emits_upper_snake_case_enum_variants() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![make_batch_status_enum_def()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config).unwrap();
    let content = result.into_iter().next().unwrap().content;

    // SHOUTY_SNAKE_CASE names must be present (matches pyo3 runtime conventions)
    assert!(
        content.contains("VALIDATING: BatchStatus = ..."),
        "stub must declare VALIDATING in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );
    assert!(
        content.contains("IN_PROGRESS: BatchStatus = ..."),
        "stub must declare IN_PROGRESS in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );
    assert!(
        content.contains("COMPLETE: BatchStatus = ..."),
        "stub must declare COMPLETE in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );

    // PascalCase names must NOT appear as attribute declarations
    assert!(
        !content.contains("Validating: BatchStatus"),
        "stub must NOT emit PascalCase variant Validating, got:\n{}",
        content
    );
    assert!(
        !content.contains("InProgress: BatchStatus"),
        "stub must NOT emit PascalCase variant InProgress, got:\n{}",
        content
    );
}

/// `.pyi` stub must escape variant names whose snake_case form collides with a Python
/// reserved keyword. `Del` snake-cases to `del`, which is a Python statement keyword and
/// produces unparseable stubs. The escape strategy appends `_` (`del_`), matching the
/// field-name escape convention via `alef::core::keywords::python_ident`.
#[test]
fn test_pyi_stub_escapes_python_keyword_variant_names() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "NodeType".to_string(),
            rust_path: "test_lib::NodeType".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Del".to_string(),
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
                },
                EnumVariant {
                    name: "Ins".to_string(),
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
                    name: "Title".to_string(),
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
            doc: String::new(),
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
    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config).unwrap();
    let content = result.into_iter().next().unwrap().content;

    // With SHOUTY_SNAKE_CASE variants (pyo3 runtime convention), Python keywords
    // and str-method collisions can't happen — DEL, INS, TITLE are all valid identifiers
    // that don't collide with reserved keywords (which are lowercase) or str methods.
    assert!(
        content.contains("DEL: NodeType = ..."),
        "stub must emit Del → DEL in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );
    assert!(
        !content.contains("del: NodeType = ..."),
        "stub must NOT emit lowercase `del` as an attribute name, got:\n{}",
        content
    );
    assert!(
        content.contains("INS: NodeType = ..."),
        "stub must emit Ins → INS in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );
    assert!(
        content.contains("TITLE: NodeType = ..."),
        "stub must emit Title → TITLE in SHOUTY_SNAKE_CASE, got:\n{}",
        content
    );
    assert!(
        !content.contains("title: NodeType = ..."),
        "stub must NOT emit the unescaped str method `title` as an attribute name, got:\n{}",
        content
    );
}

/// Opaque types that have a `[workspace.client_constructors.TypeName]` entry must emit
/// a `def __init__(self, ...) -> None: ...` stub so mypy accepts `TypeName(params...)`
/// call sites.  Without this stub mypy infers `def __init__(self) -> None` (no args)
/// and rejects every construction call site with "Too many arguments".
#[test]
fn test_opaque_type_with_constructor_emits_init_stub() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "test_lib::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "chat".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: true,
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
            doc: "Opaque client handle".to_string(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    // Config declares a constructor for DefaultClient with two params.
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.client_constructors.DefaultClient]
body = "{source_path}::new(api_key, base_url)"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "&str"

[[workspace.client_constructors.DefaultClient.params]]
name = "base_url"
type = "String"

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/src/"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok(), "stub generation must succeed");

    let content = result.unwrap().into_iter().next().unwrap().content;

    // The __init__ stub must be present with the constructor params.
    assert!(
        content.contains("def __init__(self, api_key: str, base_url: str) -> None: ..."),
        "opaque type with constructor must emit __init__ stub. Got:\n{content}"
    );

    // The ordinary instance method must still be emitted.
    assert!(
        content.contains("async def chat(self)"),
        "instance methods must still be emitted after __init__. Got:\n{content}"
    );
}

/// Opaque types WITHOUT a client_constructor entry must NOT emit a spurious __init__
/// stub (the Python default `__init__(self) -> None` is correct for them).
#[test]
fn test_opaque_type_without_constructor_omits_init_stub() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
            doc: String::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok());
    let content = result.unwrap().into_iter().next().unwrap().content;

    assert!(
        !content.contains("def __init__"),
        "opaque type without constructor must NOT emit __init__. Got:\n{content}"
    );
}

#[test]
fn test_data_enum_typed_dict_literals_use_serde_wire_names() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Action".to_string(),
            rust_path: "test_lib::Action".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "OpenURL".to_string(),
                    fields: vec![make_field("url", TypeRef::String, false)],
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
                    name: "ReadText".to_string(),
                    fields: vec![make_field("value", TypeRef::String, false)],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("read-text".to_string()),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: Some("kind".to_string()),
            serde_untagged: false,
            serde_rename_all: Some("kebab-case".to_string()),
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

    let content = backend
        .generate_type_stubs(&api, &make_config_with_stubs())
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .content;

    assert!(
        content.contains("kind: Literal[\"open-url\"]"),
        "rename_all must define TypedDict tag literals:\n{content}"
    );
    assert!(
        content.contains("kind: Literal[\"read-text\"]"),
        "serde(rename) must override rename_all:\n{content}"
    );
}

#[test]
fn test_pyi_includes_trait_bridge_registry_functions() {
    let backend = Pyo3Backend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }];
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("def register_ocr_backend(backend: object) -> None: ...")
            && content.contains("def unregister_ocr_backend(name: str) -> None: ...")
            && content.contains("def clear_ocr_backends() -> None: ..."),
        "pyi must include trait bridge functions exported by runtime:\n{content}"
    );
}

#[test]
fn test_pyi_plugin_bridge_emits_typed_protocol_and_typed_register() {
    // Neutral `Greeter` plugin trait: `process(&self, opts: &Opts) -> Doc`.
    // The host-implementable Protocol must type the struct param as its native type (`Opts`)
    // and the return as `Doc`, and `register_greeter` must take `backend: Greeter`.
    let backend = Pyo3Backend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        ..Default::default()
    }];

    let greeter = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![MethodDef {
            name: "process".to_string(),
            params: vec![ParamDef {
                name: "opts".to_string(),
                ty: TypeRef::Named("Opts".to_string()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("Doc".to_string()),
            receiver: Some(ReceiverKind::Ref),
            error_type: Some("Error".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let opts = TypeDef {
        name: "Opts".to_string(),
        rust_path: "test_lib::Opts".to_string(),
        has_serde: true,
        fields: vec![make_field("label", TypeRef::String, false)],
        ..Default::default()
    };
    let doc = TypeDef {
        name: "Doc".to_string(),
        rust_path: "test_lib::Doc".to_string(),
        has_serde: true,
        is_return_type: true,
        fields: vec![make_field("text", TypeRef::String, false)],
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![greeter, opts, doc],
        ..Default::default()
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    // (c) Plugin-pattern Protocol emitted with typed struct param + typed return.
    assert!(
        content.contains("class Greeter(Protocol):"),
        "plugin bridge must emit a host-implementable Protocol:\n{content}"
    );
    assert!(
        content.contains("def process(self, opts: Opts) -> Doc: ..."),
        "Protocol method must type the struct param as `Opts` and return as `Doc`:\n{content}"
    );

    // (d) register_* typed against the Protocol, not bare `object`.
    assert!(
        content.contains("def register_greeter(backend: Greeter) -> None: ..."),
        "register fn must type its backend param against the Protocol:\n{content}"
    );
}

#[test]
fn test_pyi_plugin_protocol_omits_defaulted_methods_and_documents_them() {
    // Plugin trait with one required and one Rust-defaulted method: the Protocol
    // must require only the former (the runtime contract) and document the latter
    // as optional — a minimal, valid backend must conform structurally.
    let backend = Pyo3Backend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        ..Default::default()
    }];

    let greeter = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![
            MethodDef {
                name: "process".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            },
            MethodDef {
                name: "supports_table_detection".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
                receiver: Some(ReceiverKind::Ref),
                has_default_impl: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![greeter],
        ..Default::default()
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("class Greeter(Protocol):"),
        "plugin bridge must emit a Protocol:\n{content}"
    );
    assert!(
        content.contains("def process(self)"),
        "required method must stay in the Protocol:\n{content}"
    );
    assert!(
        !content.contains("def supports_table_detection"),
        "Rust-defaulted method must not be a required Protocol member:\n{content}"
    );
    assert!(
        content.contains("`supports_table_detection`") && content.contains("Optional methods"),
        "defaulted method must be documented as optional:\n{content}"
    );
    assert!(
        content.contains("`initialize()`"),
        "lifecycle hooks must be documented as optional:\n{content}"
    );
}
