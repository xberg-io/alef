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

use alef_backend_magnus::MagnusBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, RubyConfig, StubsConfig};
use alef_core::ir::*;
use std::collections::HashMap;
use std::path::PathBuf;

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
    }
}

/// Helper to create a basic AlefConfig with Ruby and stubs enabled.
fn make_config_with_stubs() -> AlefConfig {
    AlefConfig {
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: Some(RubyConfig {
            gem_name: Some("test_lib".to_string()),
            stubs: Some(StubsConfig {
                output: PathBuf::from("packages/ruby/sig/"),
            }),
            features: None,
            serde_rename_all: None,
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
        }),
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
        opaque_types: HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
    }
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
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, false),
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
            doc: "Extraction configuration".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Process input with config".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Backend".to_string(),
            rust_path: "test_lib::Backend".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    doc: "Tesseract OCR".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR backend".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Available backends".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
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
        content.contains("# This file is auto-generated by alef. DO NOT EDIT."),
        "Should contain auto-generated header"
    );
    assert!(
        content.contains("# Re-generate with: alef generate"),
        "Should contain regeneration instruction"
    );

    // Check for module declaration - crate_name "test_lib" converts to "Test_lib"
    assert!(content.contains("module Test_lib"), "Should declare module Test_lib");

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

    // Check for enum stub
    assert!(content.contains("class Backend"), "Should contain Backend enum class");
    assert!(content.contains("Tesseract: Integer"), "Should have Tesseract variant");
    assert!(content.contains("PaddleOcr: Integer"), "Should have PaddleOcr variant");

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
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Processing".to_string(),
                    fields: vec![],
                    doc: "Processing status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Complete".to_string(),
                    fields: vec![],
                    doc: "Complete status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Failed".to_string(),
                    fields: vec![],
                    doc: "Failed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing status".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
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

    // Check enum class definition
    assert!(content.contains("class Status"), "Should contain Status enum class");

    // Check enum docstring
    assert!(
        content.contains("# Processing status"),
        "Should include enum documentation"
    );

    // Check all variants are defined as Integer constants
    assert!(content.contains("Pending: Integer"), "Should have Pending variant");
    assert!(
        content.contains("Processing: Integer"),
        "Should have Processing variant"
    );
    assert!(content.contains("Complete: Integer"), "Should have Complete variant");
    assert!(content.contains("Failed: Integer"), "Should have Failed variant");

    // Verify enum variants are in correct order
    let pending_idx = content.find("Pending: Integer").unwrap();
    let processing_idx = content.find("Processing: Integer").unwrap();
    let complete_idx = content.find("Complete: Integer").unwrap();
    let failed_idx = content.find("Failed: Integer").unwrap();
    assert!(
        pending_idx < processing_idx && processing_idx < complete_idx && complete_idx < failed_idx,
        "Enum variants should be in order"
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
                },
            ],
            is_opaque: true,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque processor type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
    };

    // Create config WITHOUT stubs enabled
    let config = AlefConfig {
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: Some(RubyConfig {
            gem_name: Some("test_lib".to_string()),
            stubs: None,
            features: None,
            serde_rename_all: None,
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
        }),
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
        opaque_types: HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
    };

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
            doc: "A data store".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: multiline_doc,
            cfg: None,
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            variants: vec![EnumVariant {
                name: "Fast".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            }],
            doc: "Multi-line enum doc.\nSecond line.".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
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
    };

    let config = AlefConfig {
        crate_config: CrateConfig {
            name: "my_awesome_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: Some(RubyConfig {
            gem_name: Some("my_awesome_lib".to_string()),
            stubs: Some(StubsConfig {
                output: PathBuf::from("packages/ruby/sig/"),
            }),
            features: None,
            serde_rename_all: None,
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
        }),
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
        opaque_types: HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
    };

    let result = backend.generate_type_stubs(&api, &config);

    assert!(result.is_ok());

    let files = result.unwrap();
    let rbs_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("types.rbs"))
        .unwrap();
    let content = &rbs_file.content;

    // Check for proper module naming - snake_case gets first letter capitalized only
    assert!(
        content.contains("module My_awesome_lib"),
        "Should capitalize first letter of crate name"
    );
}
