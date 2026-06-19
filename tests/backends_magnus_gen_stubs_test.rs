//! Tests for Ruby type stub (.rbs) generation via Magnus backend.
//!
//! These tests verify that the `generate_type_stubs` method correctly generates
//! RBS (Ruby Type Signatures) type stubs from API surfaces. The stubs define Ruby
//! class and method signatures for use with the built native extension.
//!
//! Test coverage:
//! - Basic stub generation with types, functions, and enums
//! - Ruby type mapping (Integer, String, Float, Array, Hash, Optional, etc.)
//! - Enum variant generation
//! - Opaque type stubs (methods only, no fields)
//! - Type stubs with both fields and methods
//! - Module naming conventions (crate_name -> module name)
//! - Graceful handling when stubs config is not enabled

use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Helper to create a FieldDef with all defaults.
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

/// Helper to create a basic ResolvedCrateConfig with Ruby and stubs enabled.
fn make_config_with_stubs() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"

[crates.ruby.stubs]
output = "packages/ruby/sig/"
"#,
    )
}

/// Helper to create a ResolvedCrateConfig with Ruby, stubs, and emit_docstrings enabled.
fn make_config_with_stubs_and_docs() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"

[crates.ruby.stubs]
output = "packages/ruby/sig/"
emit_docstrings = true
"#,
    )
}

#[test]
fn test_basic_rbs_stubs() {
    let backend = MagnusBackend;

    // Create test API surface with 1 type (2 fields), 1 function, 1 enum
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, false),
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
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
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
            error_type: None,
            doc: "Process input with config".to_string(),
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
            name: "Backend".to_string(),
            rust_path: "test_lib::Backend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    doc: "Tesseract OCR".to_string(),
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
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR backend".to_string(),
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
            doc: "Available backends".to_string(),
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

    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate at least one file");

    // Check for the types.rbs file
    let file_names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    assert!(
        file_names.iter().any(|f| f.ends_with("types.rbs")),
        "Should generate types.rbs file"
    );

    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check for auto-generated header
    assert!(
        content.contains("# This file is auto-generated by alef"),
        "Should contain auto-generated header"
    );
    assert!(
        content.contains("# To regenerate: alef generate"),
        "Should contain regeneration instruction"
    );

    // Check for module declaration — crate_name "test_lib" converts to "TestLib"
    // (PascalCase via heck::ToUpperCamelCase, valid Ruby module name).
    assert!(content.contains("module TestLib"), "Should declare module TestLib");

    // Check for type stub (Config)
    assert!(content.contains("class Config"), "Should contain Config class");
    assert!(
        content.contains("attr_reader timeout: Integer"),
        "Should have attr_reader for timeout field"
    );
    assert!(
        content.contains("attr_reader backend: String"),
        "Should have attr_reader for backend field"
    );
    assert!(
        content.contains("def initialize: (?timeout: Integer, backend: String) -> void"),
        "Should have initialize method with correct signature"
    );

    // Check for function stub
    assert!(
        content.contains("def self.process: (String input, ?Config config) -> String"),
        "Should have process function stub with correct signature"
    );

    // Check for enum stub — unit-variant enums emit a symbol literal union
    assert!(content.contains("class Backend"), "Should contain Backend enum class");
    assert!(
        content.contains("type value = :tesseract | :paddle_ocr"),
        "Should have symbol union for Backend variants"
    );

    // Check for closing module
    assert!(content.contains("end"), "Should have module closing");
}

#[test]
fn test_type_mapping_in_stubs() {
    let backend = MagnusBackend;

    // Create a type with various field types to test type mapping
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("f64_val", TypeRef::Primitive(PrimitiveType::F64), false),
                make_field("bool_val", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field("string_val", TypeRef::Optional(Box::new(TypeRef::String)), false),
                make_field("vec_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field("option_val", TypeRef::Optional(Box::new(TypeRef::String)), false),
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

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check for correct Ruby type mappings
    assert!(
        content.contains("attr_reader u32_val: Integer"),
        "u32 should map to Integer"
    );
    assert!(
        content.contains("attr_reader i64_val: Integer"),
        "i64 should map to Integer"
    );
    assert!(
        content.contains("attr_reader f64_val: Float"),
        "f64 should map to Float"
    );
    assert!(
        content.contains("attr_reader bool_val: bool"),
        "bool should map to bool"
    );
    assert!(
        content.contains("attr_reader string_val: String?"),
        "Optional<String> should map to String?"
    );
    assert!(
        content.contains("attr_reader vec_val: Array[String]"),
        "Vec<String> should map to Array[String]"
    );
    assert!(
        content.contains("attr_reader option_val: String?"),
        "Option<String> should map to String?"
    );

    // Check initialize signature contains all parameters
    assert!(
        content.contains("def initialize: (u32_val: Integer, i64_val: Integer, f64_val: Float, bool_val: bool, string_val: String?, vec_val: Array[String], option_val: String?) -> void"),
        "Initialize should have all typed parameters"
    );
}

#[test]
fn test_enum_stubs() {
    let backend = MagnusBackend;

    // Create API with a more complex enum
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
                    name: "Processing".to_string(),
                    fields: vec![],
                    doc: "Processing status".to_string(),
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
                EnumVariant {
                    name: "Failed".to_string(),
                    fields: vec![],
                    doc: "Failed status".to_string(),
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
            doc: "Processing status".to_string(),
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

    let config = make_config_with_stubs_and_docs();
    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check enum class definition
    assert!(content.contains("class Status"), "Should contain Status enum class");

    // Check enum docstring
    assert!(
        content.contains("# Processing status"),
        "Should include enum documentation"
    );

    // Unit-variant enums emit a symbol literal union in order
    assert!(
        content.contains("type value = :pending | :processing | :complete | :failed"),
        "Should have symbol union for Status variants in order"
    );
}

#[test]
fn test_opaque_type_stubs() {
    let backend = MagnusBackend;

    // Create API with an opaque type (only methods, no fields)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
                    is_static: false,
                    error_type: None,
                    doc: "Process input".to_string(),
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
                },
                MethodDef {
                    name: "new".to_string(),
                    params: vec![ParamDef {
                        name: "config".to_string(),
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
                    return_type: TypeRef::Named("Processor".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create processor".to_string(),
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
            doc: "Opaque processor type".to_string(),
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

    let config = make_config_with_stubs();
    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check for opaque type class (no fields, only methods)
    assert!(content.contains("class Processor"), "Should contain Processor class");

    // Opaque types should not have attr_reader/attr_accessor
    assert!(
        !content.contains("attr_reader") || !content[content.find("class Processor").unwrap()..].contains("attr_"),
        "Opaque types should not have field accessors"
    );

    // Check for instance method stub
    assert!(
        content.contains("def process: (String input) -> String"),
        "Should have instance method process"
    );

    // Check for static method stub
    assert!(
        content.contains("def self.new: (String config) -> Processor"),
        "Should have static method new"
    );
}

#[test]
fn test_rbs_stubs_without_config() {
    let backend = MagnusBackend;

    // Create API surface
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

    // Create config WITHOUT stubs enabled
    let config = resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
    );

    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Should handle missing stubs config gracefully");

    let files = result.unwrap();
    assert!(
        files.is_empty(),
        "Should return empty file list when stubs are not configured"
    );
}

#[test]
fn test_type_with_methods_and_fields() {
    let backend = MagnusBackend;

    // Create a type with both fields and methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Store".to_string(),
            rust_path: "test_lib::Store".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("name", TypeRef::String, false),
                make_field("count", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            methods: vec![
                MethodDef {
                    name: "get_name".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Get store name".to_string(),
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
                },
                MethodDef {
                    name: "increment".to_string(),
                    params: vec![ParamDef {
                        name: "amount".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
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
                    return_type: TypeRef::Primitive(PrimitiveType::U32),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Increment counter".to_string(),
                    receiver: Some(ReceiverKind::RefMut),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
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
            doc: "A data store".to_string(),
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

    let config = make_config_with_stubs_and_docs();
    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check for type class
    assert!(content.contains("class Store"), "Should contain Store class");

    // Check for docstring
    assert!(content.contains("# A data store"), "Should include class documentation");

    // Check for field accessors
    assert!(
        content.contains("attr_reader name: String"),
        "Should have attr_reader for name"
    );
    assert!(
        content.contains("attr_reader count: Integer"),
        "Should have attr_reader for count"
    );

    // Check for initialize method
    assert!(
        content.contains("def initialize: (name: String, count: Integer) -> void"),
        "Should have initialize with typed parameters"
    );

    // Check for instance methods
    assert!(
        content.contains("def get_name: () -> String"),
        "Should have get_name instance method"
    );
    assert!(
        content.contains("def increment: (Integer amount) -> Integer"),
        "Should have increment instance method"
    );
}

#[test]
fn test_multiline_doc_comment_is_valid_rbs() {
    let backend = MagnusBackend;

    // Multi-line doc strings must each be prefixed with `# `; raw continuation
    // lines would produce RBS syntax errors (e.g. "provider" parsed as identifier).
    let multiline_doc = "First line of the doc.\n\nSecond paragraph here.\nThird line.".to_string();

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "test_lib::Client".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
            doc: multiline_doc,
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
            rust_path: "test_lib::Mode".to_string(),
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
            methods: vec![],
            doc: "Multi-line enum doc.\nSecond line.".to_string(),
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

    let config = make_config_with_stubs_and_docs();
    let result = backend.generate_type_stubs(&api, &config);
    assert!(result.is_ok(), "Stub generation should succeed");

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // All doc text must appear only as `# ...` lines; none of the raw prose
    // should appear without a leading `#` prefix.
    let bare_prose_fragments = ["Second paragraph here.", "Third line.", "Second line."];
    for fragment in bare_prose_fragments {
        // Must not exist as a bare (un-commented) line
        for line in content.lines() {
            let trimmed = line.trim();
            assert!(trimmed != fragment, "Bare prose leaked into RBS output: {line:?}");
        }
    }

    // Positive: all doc lines should appear as `# ...`
    assert!(
        content.contains("    # First line of the doc."),
        "First doc line should be prefixed"
    );
    assert!(
        content.contains("    # Second paragraph here."),
        "Second doc line should be prefixed"
    );
    assert!(
        content.contains("    # Third line."),
        "Third doc line should be prefixed"
    );
    assert!(
        content.contains("    # Multi-line enum doc."),
        "Enum first doc line should be prefixed"
    );
    assert!(
        content.contains("    # Second line."),
        "Enum second doc line should be prefixed"
    );
}

#[test]
fn test_module_naming_from_crate_name() {
    let backend = MagnusBackend;

    // Create API with hyphenated crate name to test module naming conversion
    let api = ApiSurface {
        crate_name: "my_awesome_lib".to_string(),
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

    let config = resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my_awesome_lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "my_awesome_lib"

[crates.ruby.stubs]
output = "packages/ruby/sig/"
"#,
    );

    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok());

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // snake_case → PascalCase: my_awesome_lib → MyAwesomeLib (valid Ruby module).
    assert!(
        content.contains("module MyAwesomeLib"),
        "snake_case crate name should produce PascalCase module"
    );
}

#[test]
fn test_rbs_includes_trait_registry_functions() {
    let backend = MagnusBackend;
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
        content.contains("def self.register_ocr_backend: (untyped backend, String name) -> nil")
            && content.contains("def self.unregister_ocr_backend: (String name) -> nil")
            && content.contains("def self.clear_ocr_backends: () -> nil"),
        "RBS must include trait bridge registry functions:\n{content}"
    );
}
