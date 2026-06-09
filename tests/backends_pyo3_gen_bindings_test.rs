use alef::backends::pyo3::Pyo3Backend;
use alef::backends::pyo3::trait_bridge::{Pyo3BridgeGenerator, gen_trait_bridge};
use alef::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, PythonConfig, ResolvedCrateConfig, StubsConfig, TraitBridgeConfig};
use alef::core::ir::*;
use std::collections::HashMap;

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

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_basic_generation() {
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
                    version: Default::default(),
                },
            ],
            doc: "Processing mode".to_string(),
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

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Failed to generate bindings");
    let files = result.unwrap();

    // Should generate 1 file: lib.rs
    assert_eq!(files.len(), 1, "Expected 1 generated file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().ends_with("lib.rs"),
        "Expected lib.rs file"
    );

    let content = &lib_file.content;

    // Assert PyO3 macro markers are present
    assert!(
        content.contains("#[pyclass"),
        "Should contain #[pyclass] for Config type"
    );
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for Config methods"
    );
    assert!(
        content.contains("#[pyfunction]"),
        "Should contain #[pyfunction] for process function"
    );

    // Assert struct and enum names are present
    assert!(content.contains("struct Config"), "Should define Config struct");
    assert!(content.contains("enum Mode"), "Should define Mode enum");

    // Assert module initialization
    assert!(content.contains("#[pymodule]"), "Should contain #[pymodule] macro");
    assert!(
        content.contains("pub fn _test_lib"),
        "Should contain module init function with correct name"
    );

    // Assert pyo3 prelude import
    assert!(content.contains("use pyo3::prelude::*"), "Should import pyo3::prelude");
}

#[test]
fn public_api_converters_accept_json_string_for_dict_coercion() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructuredExtraction".to_string(),
            rust_path: "test_lib::StructuredExtraction".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("schema", TypeRef::Json, true)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        functions: vec![FunctionDef {
            name: "extract_structured".to_string(),
            rust_path: "test_lib::extract_structured".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("StructuredExtraction".to_string()),
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

    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();
    assert!(
        api_py.content.contains("import json"),
        "api.py must import json:\n{}",
        api_py.content
    );
    assert!(
        api_py
            .content
            .contains("if isinstance(value, str):\n        value = json.loads(value)"),
        "converter must parse JSON strings before dict/object coercion:\n{}",
        api_py.content
    );
}

#[test]
fn test_type_mapping() {
    let backend = Pyo3Backend;

    // TypeDef with various field types: u32, i64, String, Option<String>, Vec<String>
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "DataTypes".to_string(),
            rust_path: "test_lib::DataTypes".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("count", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("value", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("text", TypeRef::String, false),
                make_field("optional_text", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
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
            doc: "Various data types".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check that the struct is defined with expected field names
    assert!(content.contains("struct DataTypes"), "Should define DataTypes struct");

    // Verify field names are present
    assert!(content.contains("count:"), "Should have count field");
    assert!(content.contains("value:"), "Should have value field");
    assert!(content.contains("text:"), "Should have text field");
    assert!(content.contains("optional_text:"), "Should have optional_text field");
    assert!(content.contains("items:"), "Should have items field");

    // Check PyO3 derive/class macro presence
    assert!(content.contains("#[pyclass"), "Type should have #[pyclass] macro");

    // Check that conversions are generated (From/Into traits)
    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate conversion traits"
    );
}

#[test]
fn test_enum_generation() {
    let backend = Pyo3Backend;

    // EnumDef with 3 variants
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
                    version: Default::default(),
                },
            ],
            doc: "Status enum".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check enum definition
    assert!(content.contains("enum Status"), "Should define Status enum");

    // Check enum variants are present
    assert!(content.contains("Pending"), "Should have Pending variant");
    assert!(content.contains("Active"), "Should have Active variant");
    assert!(content.contains("Complete"), "Should have Complete variant");

    // Check PyO3 enum macro (should have pyclass with eq and eq_int)
    assert!(
        content.contains("#[pyclass") && content.contains("eq"),
        "Enum should have #[pyclass] with eq attribute"
    );

    // Check conversion code is generated
    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate enum conversion code"
    );
}

#[test]
fn test_generated_header() {
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // All generated files should have generated_header flag properly set
    // For lib.rs, it should be false (set by RustFileBuilder.with_generated_header())
    // but the builder adds the header comment to content
    for file in &files {
        // Check that lib.rs has the generated marker in content
        if file.path.to_string_lossy().ends_with("lib.rs") {
            // RustFileBuilder adds a header comment when .with_generated_header() is called
            assert!(
                file.content.contains("Code generated by Alef") || file.content.contains("DO NOT EDIT"),
                "Generated file should contain generation marker"
            );
        }
    }
}

#[test]
fn test_function_with_error_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "validate".to_string(),
            rust_path: "test_lib::validate".to_string(),
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
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: Some("ValidationError".to_string()),
            doc: "Validate input".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check that the function is generated with #[pyfunction]
    assert!(
        content.contains("#[pyfunction]"),
        "Function should have #[pyfunction] macro"
    );
    assert!(content.contains("fn validate"), "Should generate validate function");

    // Check that signature macro is present (PyO3 functions need signatures)
    assert!(
        content.contains("#[pyo3(signature"),
        "Function should have pyo3 signature attribute"
    );
}

#[test]
fn test_empty_api_surface() {
    let backend = Pyo3Backend;

    // Empty API surface
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Even empty API should have module init
    assert!(content.contains("#[pymodule]"), "Should contain #[pymodule] macro");
    assert!(
        content.contains("pub fn _test_lib"),
        "Should contain module init function"
    );

    // Should have PyO3 imports
    assert!(content.contains("use pyo3::prelude::*"), "Should import pyo3 prelude");
}

#[test]
fn test_module_registration() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MyType".to_string(),
            rust_path: "test_lib::MyType".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
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
            name: "get_type".to_string(),
            rust_path: "test_lib::get_type".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("MyType".to_string()),
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
        enums: vec![EnumDef {
            name: "Kind".to_string(),
            rust_path: "test_lib::Kind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "First".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Check that module init registers all types and functions
    assert!(
        content.contains("m.add_class::<MyType>"),
        "Module should register MyType class"
    );
    assert!(
        content.contains("m.add_class::<Kind>"),
        "Module should register Kind enum"
    );
    assert!(
        content.contains("m.add_function(wrap_pyfunction!(get_type"),
        "Module should register get_type function"
    );
}

#[test]
fn test_capabilities() {
    let backend = Pyo3Backend;
    let caps = backend.capabilities();

    assert!(caps.supports_async, "Should support async");
    assert!(caps.supports_classes, "Should support classes");
    assert!(caps.supports_enums, "Should support enums");
    assert!(caps.supports_option, "Should support Option types");
    assert!(caps.supports_result, "Should support Result types");
}

#[test]
fn test_language_and_name() {
    let backend = Pyo3Backend;

    assert_eq!(backend.name(), "pyo3", "Backend name should be 'pyo3'");
    assert_eq!(
        backend.language(),
        alef::core::config::Language::Python,
        "Backend language should be Python"
    );
}

#[test]
fn test_async_function() {
    let backend = Pyo3Backend;

    // FunctionDef with is_async: true
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_data".to_string(),
            rust_path: "test_lib::fetch_data".to_string(),
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
            doc: "Fetch data asynchronously".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for async function");

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert async function is marked with #[pyfunction]
    assert!(
        content.contains("#[pyfunction]"),
        "Async function should have #[pyfunction] macro"
    );
    assert!(content.contains("fn fetch_data"), "Should generate fetch_data function");

    // Assert async imports are present (needed for async functions)
    assert!(
        content.contains("pyo3_async_runtimes"),
        "Should import pyo3_async_runtimes for async support"
    );

    // Assert async runtime initialization
    assert!(
        content.contains("_tokio_runtime") || content.contains("async_runtime"),
        "Should have async runtime initialization code"
    );
}

#[test]
fn test_async_function_with_error() {
    let backend = Pyo3Backend;

    // FunctionDef with is_async: true and error_type
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_async".to_string(),
            rust_path: "test_lib::process_async".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("ProcessError".to_string()),
            doc: "Process asynchronously with error handling".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Check that PyRuntimeError import is present for error handling
    assert!(
        content.contains("PyRuntimeError"),
        "Should import PyRuntimeError for async error handling"
    );

    // Check that the function is generated
    assert!(
        content.contains("fn process_async"),
        "Should generate process_async function"
    );
}

#[test]
fn test_methods_generation() {
    let backend = Pyo3Backend;

    // TypeDef with methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![
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
                    doc: "Process some data".to_string(),
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
                    name: "reset".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Reset processor".to_string(),
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
            doc: "Test processor type".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for methods");

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert #[pymethods] block is present
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for Processor methods"
    );

    // Assert method definitions are present
    assert!(content.contains("fn process"), "Should define process method");
    assert!(content.contains("fn reset"), "Should define reset method");

    // Assert struct definition with pyclass macro
    assert!(content.contains("struct Processor"), "Should define Processor struct");
    assert!(
        content.contains("#[pyclass"),
        "Should have #[pyclass] macro on Processor"
    );
}

#[test]
fn test_async_method() {
    let backend = Pyo3Backend;

    // TypeDef with async method - must be opaque or have proper delegation setup
    // Use an opaque type so async method generation doesn't require complex conversion
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "AsyncHandler".to_string(),
            rust_path: "test_lib::AsyncHandler".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "handle_async".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Handle asynchronously".to_string(),
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
            is_opaque: true, // Make it opaque so async delegation works
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Check that async method is defined
    assert!(content.contains("fn handle_async"), "Should define async method");

    // Check async runtime imports
    assert!(
        content.contains("pyo3_async_runtimes"),
        "Should import pyo3_async_runtimes for async methods"
    );

    // Check that future_into_py is used for async handling
    assert!(
        content.contains("future_into_py"),
        "Should use future_into_py for async methods"
    );
}

#[test]
fn test_error_types() {
    let backend = Pyo3Backend;

    // API surface with ErrorDef
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    message_template: Some("not found".to_string()),
                    doc: "Item not found".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![],
                    message_template: Some("invalid input".to_string()),
                    doc: "Invalid input provided".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: "Error type for processing".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for error types");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert error creation code (create_exception! macros)
    assert!(
        content.contains("create_exception!"),
        "Should generate create_exception! macros for error types"
    );

    // Assert that specific error variants are created
    assert!(
        content.contains("NotFoundError"),
        "Should create NotFoundError exception"
    );
    assert!(
        content.contains("InvalidInputError"),
        "Should create InvalidInputError exception"
    );
    assert!(
        content.contains("ProcessError"),
        "Should create ProcessError base exception"
    );

    // Assert error converter function is generated
    assert!(
        content.contains("process_error_to_py_err") || content.contains("_to_py_err"),
        "Should generate error converter function"
    );
}

#[test]
fn test_opaque_type() {
    let backend = Pyo3Backend;

    // TypeDef with is_opaque: true
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
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "An opaque handle type".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for opaque type");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert opaque struct is generated with Arc<inner>
    assert!(
        content.contains("struct OpaqueHandle"),
        "Should define OpaqueHandle struct"
    );
    assert!(content.contains("Arc<"), "Opaque type should use Arc wrapper");
    assert!(content.contains("inner:"), "Opaque type should have inner field");

    // Assert Arc import is present
    assert!(content.contains("std::sync::Arc"), "Should import Arc for opaque types");

    // Assert pyclass macro is present
    assert!(
        content.contains("#[pyclass"),
        "Opaque type should have #[pyclass] macro"
    );
}

#[test]
fn test_optional_and_vec_fields() {
    let backend = Pyo3Backend;

    // TypeDef with Optional and Vec fields
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Container".to_string(),
            rust_path: "test_lib::Container".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("optional_text", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field(
                    "optional_numbers",
                    TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I64))))),
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
            doc: "Container with optional and vec fields".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for optional/vec fields");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert struct is defined
    assert!(content.contains("struct Container"), "Should define Container struct");

    // Assert field names are present
    assert!(content.contains("optional_text:"), "Should have optional_text field");
    assert!(content.contains("items:"), "Should have items field");
    assert!(
        content.contains("optional_numbers:"),
        "Should have optional_numbers field"
    );

    // Assert pyclass macro
    assert!(content.contains("#[pyclass"), "Type should have #[pyclass] macro");

    // Assert Vec conversion code or container types are present
    assert!(
        content.contains("Vec") || content.contains("From") || content.contains("Into"),
        "Should handle Vec and Option conversions"
    );
}

#[test]
fn test_static_method() {
    let backend = Pyo3Backend;

    // TypeDef with static method
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Factory".to_string(),
            rust_path: "test_lib::Factory".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "create_default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("Factory".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: "Create a default Factory".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert static method is defined
    assert!(content.contains("fn create_default"), "Should define static method");

    // Assert #[pymethods] block is present
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for static methods"
    );

    // Assert staticmethod attribute (part of PyO3 static method binding)
    assert!(
        content.contains("staticmethod") || content.contains("create_default"),
        "Should mark method as static or generate appropriately"
    );
}

#[test]
fn test_exceptions_py_classes_without_docs_have_generated_docstrings() {
    let backend = Pyo3Backend;

    // Errors with no docstrings — exception classes must have generated docstrings (D101).
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "SampleLlmError".to_string(),
            rust_path: "test_lib::SampleLlmError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "AuthenticationError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "RateLimitedError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
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

    let config = make_config();

    let result = backend.generate_public_api(&api, &config);
    assert!(result.is_ok(), "Failed to generate public API");

    let files = result.unwrap();
    let exceptions_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("exceptions.py"))
        .expect("exceptions.py should be generated");

    let content = &exceptions_file.content;

    // No class should use `pass` — all must have docstrings (ruff D101).
    assert!(
        !content.contains("    pass"),
        "Exception classes must use docstrings, not `pass`"
    );

    // The base error class should have a generated docstring from its name.
    assert!(
        content.contains("\"\"\"Sample llm error.\"\"\""),
        "SampleLlmError should have generated docstring"
    );

    // Variant classes should also have generated docstrings.
    assert!(
        content.contains("\"\"\"Authentication error.\"\"\""),
        "AuthenticationError should have generated docstring"
    );
    assert!(
        content.contains("\"\"\"Rate limited error.\"\"\""),
        "RateLimitedError should have generated docstring"
    );

    // Verify no empty class body (class header immediately followed by blank line).
    for (i, line) in content.lines().enumerate() {
        if line.starts_with("class ") {
            let next_non_empty = content.lines().skip(i + 1).find(|l| !l.trim().is_empty());
            assert!(
                next_non_empty.is_none_or(|l| l.trim() != ""),
                "Class at line {} has empty body",
                i + 1
            );
        }
    }
}

/// Regression test for sample_crate-dev/alef#1 / sample_crate-dev/sample_crate#310.
///
/// A type with both `has_default = true` AND `is_return_type = true` (e.g. `ParseOutput`)
/// must be re-exported in `__init__.py` from the native Rust module, NOT from `options.py`.
/// `options.py` must NOT emit a `@dataclass` shadow class for such types; the authoritative
/// definition lives in the native module as a `#[pyclass]` struct. The shadow class caused
/// static analysis tools (Pylance) to report a type mismatch because the two classes are
/// unrelated even though they share a name.
#[test]
fn test_return_type_exported_from_native_module_not_options() {
    let backend = Pyo3Backend;

    // ParseOutput: has_default=true (implements Default), is_return_type=true (returned by convert())
    // ParseOptions: has_default=true, is_return_type=false (input/config type)
    let conversion_result = TypeDef {
        name: "ParseOutput".to_string(),
        rust_path: "my_lib::ParseOutput".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("content", TypeRef::String, false),
            make_field("title", TypeRef::Optional(Box::new(TypeRef::String)), true),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Result of a conversion operation.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let conversion_options = TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), false)],
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
        doc: "Options for conversion.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![conversion_result, conversion_options],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
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
            return_type: TypeRef::Named("ParseOutput".to_string()),
            is_async: false,
            error_type: None,
            doc: "Convert input to markdown.".to_string(),
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

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_my_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/my_lib"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");
    let options_py = files
        .iter()
        .find(|f| f.path.ends_with("options.py"))
        .expect("options.py not generated");

    // ParseOutput (return type) must be imported from the native module.
    let native_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from ._my_lib import"))
        .unwrap_or("");
    assert!(
        native_import_line.contains("ParseOutput"),
        "__init__.py must import ParseOutput from the native module, got:\n{}",
        init_py.content
    );

    // ParseOutput must NOT appear in the .options import.
    let options_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from .options import"))
        .unwrap_or("");
    assert!(
        !options_import_line.contains("ParseOutput"),
        "__init__.py must not import ParseOutput from .options, got:\n{}",
        init_py.content
    );

    // ParseOptions (config/input type) must still be imported from .options.
    assert!(
        options_import_line.contains("ParseOptions"),
        "__init__.py must import ParseOptions from .options, got:\n{}",
        init_py.content
    );

    // Both names must appear in __all__.
    assert!(
        init_py.content.contains("\"ParseOutput\""),
        "__init__.py __all__ must include ParseOutput, got:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("\"ParseOptions\""),
        "__init__.py __all__ must include ParseOptions, got:\n{}",
        init_py.content
    );

    // options.py must NOT define a @dataclass shadow for ParseOutput.
    assert!(
        !options_py.content.contains("class ParseOutput"),
        "options.py must not define a ParseOutput shadow class, got:\n{}",
        options_py.content
    );

    // options.py MUST still define ParseOptions (the input/config type).
    assert!(
        options_py.content.contains("class ParseOptions"),
        "options.py must still define ParseOptions dataclass, got:\n{}",
        options_py.content
    );
}

#[test]
fn test_api_py_imports_config_dto_with_self_returning_method_from_options() {
    // Regression: alef#72. A has_default config DTO that exposes a builder method
    // returning `Self` (e.g. `PackConfig::from_toml_file -> PackConfig`) must still
    // be imported from `.options` in api.py, not from `._native`. The pre-fix code
    // walked method return types into `return_type_names`, which incorrectly pulled
    // self-builders out of the options classification.
    let backend = Pyo3Backend;

    // ParseOutput: return type of free function `convert` — stays on ._native.
    let conversion_result = TypeDef {
        name: "ParseOutput".to_string(),
        rust_path: "my_lib::ParseOutput".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("content", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: true,
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
    };

    // ParseOptions: input/config DTO with `Self`-returning builder methods.
    // This is the regression: before the fix, the method returns caused this type
    // to be excluded from options_type_names.
    let with_verbose = MethodDef {
        name: "with_verbose".to_string(),
        params: vec![make_param_def(
            "verbose",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
        )],
        return_type: TypeRef::Named("ParseOptions".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let default_method = MethodDef {
        name: "default".to_string(),
        params: vec![],
        return_type: TypeRef::Named("ParseOptions".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let conversion_options = TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), false)],
        methods: vec![with_verbose, default_method],
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
    };

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![conversion_result, conversion_options],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
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
                    name: "options".to_string(),
                    ty: TypeRef::Named("ParseOptions".to_string()),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
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
            return_type: TypeRef::Named("ParseOutput".to_string()),
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

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_my_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/my_lib"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    let native_import_line = api_py
        .content
        .lines()
        .find(|l| l.contains("from ._my_lib import"))
        .unwrap_or("");
    let options_import_line = api_py
        .content
        .lines()
        .find(|l| l.contains("from .options import"))
        .unwrap_or("");

    // ParseOptions has Self-returning methods, so the pre-fix code put it in
    // return_type_names and excluded it from options_type_names. Verify the fix.
    assert!(
        options_import_line.contains("ParseOptions"),
        "api.py must import ParseOptions from .options, got native={:?} options={:?}\n\nFull api.py:\n{}",
        native_import_line,
        options_import_line,
        api_py.content
    );
    assert!(
        !native_import_line.contains("ParseOptions"),
        "api.py must NOT import ParseOptions from ._my_lib, got native={:?}\n\nFull api.py:\n{}",
        native_import_line,
        api_py.content
    );

    // Regression boundary: ParseOutput IS a free-function return type, so it
    // must continue to come from the native module.
    assert!(
        native_import_line.contains("ParseOutput"),
        "api.py must import ParseOutput from ._my_lib, got native={:?}\n\nFull api.py:\n{}",
        native_import_line,
        api_py.content
    );
    assert!(
        !options_import_line.contains("ParseOutput"),
        "api.py must NOT import ParseOutput from .options, got options={:?}\n\nFull api.py:\n{}",
        options_import_line,
        api_py.content
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_method_def(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    has_error: bool,
    has_default_impl: bool,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: format!("Documentation for {name}."),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl,
        trait_source: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_param_def(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_bridge_generator(core_import: &str) -> Pyo3BridgeGenerator {
    Pyo3BridgeGenerator {
        core_import: core_import.to_string(),
        type_paths: HashMap::new(),
        error_type: "Error".to_string(),
    }
}

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

fn make_api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// gen_sync_method_body
// ---------------------------------------------------------------------------

#[test]
fn test_gen_sync_method_body_unit_return_no_error() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("tick", vec![], TypeRef::Unit, false, false, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(body.contains("Python::attach"), "sync body should use Python::attach");
    assert!(
        body.contains("call_method0(\"tick\")"),
        "should call Python method by name with no args"
    );
    assert!(
        body.contains("unwrap_or(())"),
        "unit return without error should use unwrap_or(())"
    );
}

#[test]
fn test_gen_sync_method_body_string_return_no_error() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("name", vec![], TypeRef::String, false, false, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(body.contains("call_method0(\"name\")"), "should call method by name");
    assert!(body.contains("extract::<String>()"), "should extract String return");
    assert!(
        body.contains("unwrap_or_default()"),
        "infallible string return should use unwrap_or_default"
    );
}

#[test]
fn test_gen_sync_method_body_with_params_uses_call_method1() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def(
        "process",
        vec![make_param_def("input", TypeRef::String, false)],
        TypeRef::String,
        false,
        false,
        false,
    );
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("call_method1(\"process\""),
        "single-param method should use call_method1"
    );
}

#[test]
fn test_gen_sync_method_body_with_error_uses_map_err() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("run", vec![], TypeRef::Unit, false, true, false);
    let body = generator.gen_sync_method_body(&method, &spec);

    assert!(
        body.contains("map_err"),
        "fallible method should have map_err for error conversion"
    );
    assert!(
        body.contains("Error::from("),
        "error path should call the configured error_constructor"
    );
}

// ---------------------------------------------------------------------------
// gen_async_method_body
// ---------------------------------------------------------------------------

#[test]
fn test_gen_async_method_body_uses_spawn_blocking() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("fetch", vec![], TypeRef::String, true, true, false);
    let body = generator.gen_async_method_body(&method, &spec);

    assert!(
        body.contains("spawn_blocking"),
        "async method should use spawn_blocking for Python dispatch"
    );
    assert!(
        body.contains("Python::attach"),
        "async body should re-enter Python GIL inside spawn_blocking"
    );
    assert!(
        body.contains(".await"),
        "async body should await the spawn_blocking result"
    );
}

#[test]
fn test_gen_async_method_body_clones_ref_params() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def(
        "transform",
        vec![make_param_def("data", TypeRef::String, false)],
        TypeRef::String,
        true,
        true,
        false,
    );
    let body = generator.gen_async_method_body(&method, &spec);

    // owned params must be cloned before the blocking closure captures them
    assert!(
        body.contains("let data = data.clone()"),
        "owned params should be cloned before spawn_blocking capture"
    );
}

#[test]
fn test_gen_async_method_body_unit_return() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = make_bridge_cfg("MyTrait");
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let method = make_method_def("shutdown", vec![], TypeRef::Unit, true, true, false);
    let body = generator.gen_async_method_body(&method, &spec);

    assert!(
        body.contains("map(|_| ())"),
        "async unit return should map result to ()"
    );
    assert!(
        body.contains("Error::from("),
        "async unit return error path should call the configured error_constructor"
    );
}

// ---------------------------------------------------------------------------
// gen_registration_fn
// ---------------------------------------------------------------------------

#[test]
fn test_gen_registration_fn_requires_register_fn_and_registry_getter() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    // Neither register_fn nor registry_getter: should produce empty string
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);
    assert!(
        out.is_empty(),
        "registration fn should be empty when register_fn is absent"
    );
}

#[test]
fn test_gen_registration_fn_validates_required_methods() {
    let generator = make_bridge_generator("my_lib");
    let required_method = make_method_def("process", vec![], TypeRef::String, false, true, false);
    let optional_method = make_method_def("describe", vec![], TypeRef::String, false, false, true);
    let trait_def = make_trait_def("Backend", "my_lib::Backend", vec![required_method, optional_method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Backend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);

    // The registration function must validate all required methods are present
    assert!(
        out.contains("\"process\""),
        "registration fn should validate required method 'process'"
    );
    // Optional method should also appear in required_methods list (it's still listed)
    assert!(
        out.contains("PyAttributeError"),
        "registration fn should raise PyAttributeError for missing methods"
    );
    assert!(
        out.contains("#[pyfunction]"),
        "registration fn should be annotated with #[pyfunction]"
    );
    assert!(
        out.contains("fn register_backend"),
        "registration fn should use the configured name"
    );
    assert!(out.contains("Arc::new(wrapper)"), "registration fn should wrap in Arc");
}

#[test]
fn test_gen_registration_fn_calls_registry_getter() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "Processor",
        "my_lib::Processor",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Processor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::registry::get_processors".to_string()),
        register_fn: Some("register_processor".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);

    assert!(
        out.contains("my_lib::registry::get_processors()"),
        "registration fn should call the configured registry getter"
    );
    assert!(
        out.contains("registry.register(arc)"),
        "registration fn should call registry.register"
    );
    assert!(
        out.contains("registry.write()"),
        "registration fn should acquire write lock"
    );
}

#[test]
fn test_gen_unregistration_fn_emits_typed_pyfunction_when_configured() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "TextBackend",
        "my_lib::TextBackend",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::plugins::registry::get_text_backend_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),
        unregister_fn: Some("unregister_text_backend".to_string()),
        clear_fn: Some("clear_text_backends".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let unreg = generator.gen_unregistration_fn(&spec);
    assert!(unreg.contains("#[pyfunction]"), "unreg must be a pyfunction: {unreg}");
    assert!(unreg.contains("name: String"), "unreg takes name as String: {unreg}");
    assert!(
        unreg.contains("my_lib::plugins::text_backend::unregister_text_backend"),
        "unreg must call the host plugin module fn: {unreg}"
    );

    let clear = generator.gen_clear_fn(&spec);
    assert!(clear.contains("#[pyfunction]"), "clear must be a pyfunction: {clear}");
    assert!(
        clear.contains("my_lib::plugins::text_backend::clear_text_backends"),
        "clear must call the host plugin module fn: {clear}"
    );
}

#[test]
fn test_gen_unregistration_fn_returns_empty_when_unset() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "TextBackend",
        "my_lib::TextBackend",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: Some("register_text_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    assert!(generator.gen_unregistration_fn(&spec).is_empty());
    assert!(generator.gen_clear_fn(&spec).is_empty());
}

// ---------------------------------------------------------------------------
// gen_trait_bridge (the main entry point)
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_produces_non_empty_output_for_plugin_pattern() {
    let method = make_method_def("process", vec![], TypeRef::String, false, true, false);
    let trait_def = make_trait_def("TextBackend", "my_lib::TextBackend", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(!code.code.is_empty(), "gen_trait_bridge must produce non-empty output");
    assert!(
        code.code.contains("PyTextBackendBridge"),
        "output should define the bridge wrapper struct"
    );
    assert!(
        code.imports.iter().any(|i| i.contains("pyo3::prelude")),
        "output should import pyo3 prelude"
    );
    assert!(
        code.code.contains("fn process"),
        "output should include the trait method"
    );
}

#[test]
fn test_gen_trait_bridge_wrapper_struct_has_required_fields() {
    let method = make_method_def("run", vec![], TypeRef::Unit, false, true, false);
    let trait_def = make_trait_def("Worker", "my_lib::Worker", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Worker".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_workers".to_string()),
        register_fn: Some("register_worker".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // The wrapper struct must hold the Python object and a cached name field
    assert!(
        code.code.contains("inner: Py<PyAny>"),
        "wrapper struct must hold inner Py<PyAny>"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "wrapper struct must hold cached_name"
    );
}

#[test]
fn test_gen_trait_bridge_generates_registration_fn_when_configured() {
    let method = make_method_def("infer", vec![], TypeRef::String, false, true, false);
    let trait_def = make_trait_def("InferenceBackend", "my_lib::InferenceBackend", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "InferenceBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_inference_registry".to_string()),
        register_fn: Some("register_inference_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("fn register_inference_backend"),
        "should generate registration function with configured name"
    );
    assert!(
        code.code.contains("#[pyfunction]"),
        "registration function should carry #[pyfunction] attribute"
    );
}

#[test]
fn test_gen_trait_bridge_with_sync_and_async_required_methods() {
    // A trait with one sync and one async required method — exercises both code paths
    let sync_method = make_method_def(
        "validate",
        vec![],
        TypeRef::Primitive(PrimitiveType::Bool),
        false,
        false,
        false,
    );
    let async_method = make_method_def("process", vec![], TypeRef::String, true, true, false);
    let trait_def = make_trait_def(
        "HybridBackend",
        "my_lib::HybridBackend",
        vec![sync_method, async_method],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "HybridBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_hybrid_registry".to_string()),
        register_fn: Some("register_hybrid_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(!code.code.is_empty(), "output must not be empty");
    // Sync method body uses Python::attach (no spawn_blocking)
    assert!(
        code.code.contains("fn validate"),
        "sync method should be present in trait impl"
    );
    // Async method body uses spawn_blocking
    assert!(
        code.code.contains("fn process"),
        "async method should be present in trait impl"
    );
    assert!(
        code.code.contains("spawn_blocking"),
        "async method body should use spawn_blocking"
    );
    // Both methods are required — registration fn should validate both
    assert!(
        code.code.contains("\"validate\"") || code.code.contains("\"process\""),
        "registration fn should validate required method names"
    );
}

/// Regression test: a non-opaque struct with a static `default()` method that returns
/// `TypeRef::Named` with the same name as the struct must wrap the core call with `.into()`.
///
/// Before the fix, `wrap_return_with_mutex` had a guard `if n == type_name { expr }` that
/// silently skipped the conversion, producing code like:
///   `fn default() -> ParseOptions { core::ParseOptions::default() }`
/// which fails to compile because the body returns the core type, not the binding wrapper.
#[test]
fn test_static_default_returns_binding_wrapper_not_core_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::options::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
            methods: vec![MethodDef {
                name: "default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("ParseOptions".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: String::new(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must call the core default() and convert with .into() so the
    // binding wrapper type is returned, not the bare inner core type.
    assert!(
        content.contains("test_lib::options::ParseOptions::default().into()"),
        "static default() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn default:\n{}",
        extract_fn_snippet(content, "fn default")
    );
}

/// Regression test: a static `from_update()` method on a non-opaque struct that takes a
/// `Named` param and returns `TypeRef::Named` with the same struct name must also end with
/// `.into()` so the core return value is converted to the binding wrapper.
#[test]
fn test_static_from_update_returns_binding_wrapper_not_core_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::options::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
                methods: vec![MethodDef {
                    name: "from_update".to_string(),
                    params: vec![ParamDef {
                        name: "update".to_string(),
                        ty: TypeRef::Named("ParseOptionsUpdate".to_string()),
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
                    return_type: TypeRef::Named("ParseOptions".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: String::new(),
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
            },
            TypeDef {
                name: "ParseOptionsUpdate".to_string(),
                rust_path: "test_lib::ParseOptionsUpdate".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field(
                    "enabled",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                    true,
                )],
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must delegate to the core method and convert the result with .into().
    assert!(
        content.contains("ParseOptions::from_update(update_core).into()"),
        "static from_update() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn from_update:\n{}",
        extract_fn_snippet(content, "fn from_update")
    );
}

/// Extract a ~200-char snippet around the first occurrence of `marker` for assertion messages.
fn extract_fn_snippet<'a>(content: &'a str, marker: &str) -> &'a str {
    let start = content.find(marker).unwrap_or(0);
    let end = (start + 200).min(content.len());
    &content[start..end]
}

// ---------------------------------------------------------------------------
// Tests for alef#44 fixes
// ---------------------------------------------------------------------------

/// Item 1 — `#[serde(skip)]` must be emitted for sanitized fields.
///
/// A field like `cancel_token: String` (sanitized from `CancellationToken`) must carry
/// `#[serde(skip)]` in the generated Rust binding struct so that JSON round-trips do not
/// include the field and cause "unknown field 'cancel_token'" errors at runtime.
#[test]
fn test_sanitized_field_gets_serde_skip() {
    let backend = Pyo3Backend;

    let mut cancel_field = make_field("cancel_token", TypeRef::String, true);
    cancel_field.sanitized = true;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false),
                cancel_field,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_file = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let content = &lib_file.content;

    // The sanitized field must be tagged with #[serde(skip)] so JSON round-trips skip it.
    assert!(
        content.contains("serde(skip)"),
        "sanitized cancel_token field must get #[serde(skip)];\ncontent:\n{}",
        content
    );
}

/// Item 2 — Non-`Option` enum fields must not fall back to `String::default()` (`""`).
///
/// When a struct field's type is sanitized to `String` (e.g. `result_format: OutputFormat`
/// where `OutputFormat` was unknown to the extractor), the generated binding stores it as
/// `result_format: String`. Serde deserialization of `{"result_format": ""}` (the
/// `String::default()`) would fail with "unknown variant ''". The `#[serde(skip)]` fix
/// ensures the field is excluded from JSON, so its Rust `Default::default()` (`""`) is
/// used silently — avoiding the failure. This test verifies that a sanitized `String`
/// field in a has_default struct gets `#[serde(skip)]`.
#[test]
fn test_sanitized_enum_like_field_gets_serde_skip() {
    let backend = Pyo3Backend;

    // Simulate OutputFormat sanitized to String (extractor could not resolve the enum type)
    let mut format_field = make_field("result_format", TypeRef::String, false);
    format_field.sanitized = true;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false),
                format_field,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_file = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib_file.content.contains("serde(skip)"),
        "sanitized result_format field must get #[serde(skip)] to avoid unknown-variant errors;\ncontent:\n{}",
        lib_file.content
    );
}

/// Item 3 — `api.py` wrapper must forward arguments by keyword, not positional.
///
/// The pyo3 signature order (required first, optional second) may differ from the
/// Python wrapper function signature. Forwarding by keyword ensures slot alignment
/// regardless of declaration order.
#[test]
fn test_api_py_uses_keyword_arguments() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
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
                    name: "mime_type".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
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
            doc: "Extract file.".to_string(),
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

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();

    // The call to _rust.extract_file must use keyword arguments.
    assert!(
        api_py.content.contains("path=path"),
        "api.py must forward path by keyword;\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("mime_type=mime_type"),
        "api.py must forward mime_type by keyword;\ncontent:\n{}",
        api_py.content
    );
    // Must NOT use raw positional call like `_rust.extract_file(path, mime_type, config)`
    assert!(
        !api_py.content.contains("_rust.extract_file(path, "),
        "api.py must not use positional arguments for extract_file;\ncontent:\n{}",
        api_py.content
    );
}

/// Item 4 — Async pyo3 functions must produce `async def` + `await` wrappers in `api.py`.
///
/// Pyo3 async functions return coroutines. The Python wrapper must be `async def` and
/// must `await` the native call so callers can use `await extract_file(...)`.
#[test]
fn test_async_function_emits_async_def_and_await() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_bytes".to_string(),
            rust_path: "test_lib::extract_bytes".to_string(),
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
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: "Extract bytes.".to_string(),
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

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();

    assert!(
        api_py.content.contains("async def extract_bytes"),
        "api.py async function must use 'async def';\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("await _rust.extract_bytes"),
        "api.py async function must await the native call;\ncontent:\n{}",
        api_py.content
    );
    // Must NOT be a plain sync def
    assert!(
        !api_py.content.contains("\ndef extract_bytes"),
        "api.py async function must NOT use plain 'def';\ncontent:\n{}",
        api_py.content
    );
}

/// Item 5 — Trait-bridge `register_*` helpers must appear in `api.py` and `__init__.py` `__all__`.
///
/// `register_embedding_backend` and `register_text_backend` are emitted as `#[pyfunction]`
/// by trait_bridge codegen and added to the pyo3 module, but they are not in `api.functions`.
/// They must be re-exported through `api.py` and listed in `__all__` so callers can use
/// `sample_crate.register_text_backend(...)` instead of `sample_crate._sample_crate.register_text_backend(...)`.
#[test]
fn test_trait_bridge_register_fns_in_api_py_and_all() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "path".to_string(),
                ty: TypeRef::Path,
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
            doc: "Extract.".to_string(),
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

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });
    // Configure two trait bridges with register_fn
    config.trait_bridges = vec![
        TraitBridgeConfig {
            trait_name: "TextBackend".to_string(),
            super_trait: None,
            registry_getter: Some("test_lib::get_ocr_registry".to_string()),
            register_fn: Some("register_text_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        },
        TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: None,
            registry_getter: Some("test_lib::get_embedding_registry".to_string()),
            register_fn: Some("register_embedding_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        },
    ];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();
    let init_py = files.iter().find(|f| f.path.ends_with("__init__.py")).unwrap();

    // api.py must contain pass-through wrappers for both register_* functions
    assert!(
        api_py.content.contains("def register_text_backend"),
        "api.py must contain register_text_backend wrapper;\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("def register_embedding_backend"),
        "api.py must contain register_embedding_backend wrapper;\ncontent:\n{}",
        api_py.content
    );

    // __init__.py must re-export them from .api
    assert!(
        init_py.content.contains("register_text_backend"),
        "__init__.py must import register_text_backend from .api;\ncontent:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("register_embedding_backend"),
        "__init__.py must import register_embedding_backend from .api;\ncontent:\n{}",
        init_py.content
    );

    // Both must appear in __all__
    assert!(
        init_py.content.contains("\"register_text_backend\""),
        "__init__.py __all__ must include register_text_backend;\ncontent:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("\"register_embedding_backend\""),
        "__init__.py __all__ must include register_embedding_backend;\ncontent:\n{}",
        init_py.content
    );
}

#[test]
fn test_options_py_does_not_import_data_enum_aliases_at_runtime() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructureItem".to_string(),
            rust_path: "test_lib::StructureItem".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("kind", TypeRef::Named("StructureKind".to_string()), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A structural item.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "StructureKind".to_string(),
            rust_path: "test_lib::StructureKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Function".to_string(),
                fields: vec![make_field("name", TypeRef::String, false)],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            }],
            doc: "The kind of structural item.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("type".to_string()),
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
    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate public API");
    let options_py = files.iter().find(|f| f.path.ends_with("options.py")).unwrap();

    assert!(
        !options_py
            .content
            .contains("from ._test_lib import (\n    StructureKind,"),
        "data enum aliases must not be imported from the native module and then redefined;\ncontent:\n{}",
        options_py.content
    );
    assert!(
        options_py.content.contains("StructureKind = str"),
        "data enum alias should still be emitted for Python-side annotations;\ncontent:\n{}",
        options_py.content
    );
}

/// capsule_types wires up PyCapsule pass-through end-to-end:
/// - The Language type does NOT get a #[pyclass] wrapper.
/// - get_language returns via PyCapsule_New (capsule round-trip).
/// - get_parser constructs via py.import("sample_language").getattr("Parser").call1.
#[test]
fn test_capsule_types_end_to_end() {
    use alef::core::config::CapsuleTypeConfig;

    let backend = Pyo3Backend;

    // IR: two opaque types that are listed as capsule types + two functions.
    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            // Language — capsule round-trip type
            TypeDef {
                name: "Language".to_string(),
                rust_path: "sample_pack::Language".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "A sample_language Language handle.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Parser — ConstructFrom type (no into_raw; built via sample_language.Parser(language))
            TypeDef {
                name: "Parser".to_string(),
                rust_path: "sample_pack::Parser".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "A sample_language Parser.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![
            // get_language(name: &str) -> Result<Language, Error>
            FunctionDef {
                name: "get_language".to_string(),
                rust_path: "sample_pack::get_language".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "name".to_string(),
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
                return_type: TypeRef::Named("Language".to_string()),
                is_async: false,
                error_type: Some("sample_pack::Error".to_string()),
                doc: "Look up a language by name.".to_string(),
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
            // get_parser(name: &str) -> Result<Parser, Error>
            FunctionDef {
                name: "get_parser".to_string(),
                rust_path: "sample_pack::get_parser".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "name".to_string(),
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
                return_type: TypeRef::Named("Parser".to_string()),
                is_async: false,
                error_type: Some("sample_pack::Error".to_string()),
                doc: "Get a parser for a language by name.".to_string(),
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
        enums: vec![],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "sample_pack::Error".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".to_string(),
                message_template: Some("language not found: {0}".to_string()),
                fields: vec![make_field("msg", TypeRef::String, false)],
                has_source: false,
                has_from: false,
                is_unit: false,
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

    let mut config = make_config();
    let mut capsule_map: HashMap<String, CapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        CapsuleTypeConfig::Capsule("sample_language.Language".to_string()),
    );
    capsule_map.insert(
        "Parser".to_string(),
        CapsuleTypeConfig::ConstructFrom {
            python_type: "sample_language.Parser".to_string(),
            construct_from: "Language".to_string(),
        },
    );
    config.python = Some(PythonConfig {
        module_name: Some("_sample_pack".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: None,
        features: None,
        serde_rename_all: None,
        capsule_types: capsule_map,
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Language and Parser must NOT appear as #[pyclass] opaque wrappers.
    assert!(
        !content.contains("struct Language"),
        "Language must not be emitted as a #[pyclass] struct; content:\n{content}"
    );
    assert!(
        !content.contains("struct Parser"),
        "Parser must not be emitted as a #[pyclass] struct; content:\n{content}"
    );

    // get_language must use PyCapsule_New for the capsule round-trip return.
    assert!(
        content.contains("PyCapsule_New"),
        "get_language must call PyCapsule_New; content:\n{content}"
    );

    // get_parser must import sample_language and call Parser via getattr + call1.
    assert!(
        content.contains("py.import(\"sample_language\")"),
        "get_parser must import the sample_language module; content:\n{content}"
    );
    assert!(
        content.contains("getattr(\"Parser\")"),
        "get_parser must call getattr(\"Parser\"); content:\n{content}"
    );
    assert!(
        content.contains("call1("),
        "get_parser must call call1 to construct the Parser; content:\n{content}"
    );

    // The preamble must suppress unsafe_code so downstreams with
    // workspace-level `unsafe_code = "deny"` compile without overrides.
    assert!(
        content.contains("allow(unsafe_code)"),
        "preamble must include #![allow(unsafe_code)]; content:\n{content}"
    );

    // Bug 1 — error_converter_name must emit function-ref, not redundant closure.
    // With Error in the IR, error_to_py_err is a known converter; it must appear as
    // `.map_err(error_to_py_err)`, NOT `.map_err(|e| error_to_py_err(e))`.
    assert!(
        content.contains(".map_err(error_to_py_err)"),
        "lib.rs must use .map_err(error_to_py_err) (function ref, not closure); content:\n{content}"
    );
    assert!(
        !content.contains(".map_err(|e| error_to_py_err(e))"),
        "lib.rs must NOT contain redundant closure .map_err(|e| error_to_py_err(e)); content:\n{content}"
    );

    // Bugs 2 and 3 — api.py import order and capsule type imports.
    let pub_files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api with capsule_types should succeed");
    let api_py = pub_files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");
    let api_py_content = &api_py.content;

    // Bug 2: stdlib `from typing import` must appear BEFORE any `from .` local imports.
    let typing_pos = api_py_content
        .find("from typing import")
        .expect("api.py must contain 'from typing import'");
    let first_local_pos = api_py_content.find("from .").unwrap_or(api_py_content.len());
    assert!(
        typing_pos < first_local_pos,
        "api.py: 'from typing import' must come before 'from .' imports (isort I001);\ncontent:\n{api_py_content}"
    );

    // Bug 3: capsule types must have an explicit import so bare names resolve (ruff F821).
    // Both Language (sample_language.Language) and Parser (sample_language.Parser) share the
    // `sample_language` module, so a single `from sample_language import Language, Parser` is expected.
    assert!(
        api_py_content.contains("from sample_language import"),
        "api.py must contain 'from sample_language import' for capsule types; content:\n{api_py_content}"
    );
    assert!(
        api_py_content.contains("Language"),
        "api.py capsule import must include Language; content:\n{api_py_content}"
    );
    assert!(
        api_py_content.contains("Parser"),
        "api.py capsule import must include Parser; content:\n{api_py_content}"
    );
    // Capsule types must NOT be imported from ._native (they have no #[pyclass] there).
    let native_import_line = api_py_content
        .lines()
        .find(|l| l.contains("from ._sample_pack import") || l.contains("from ._native import"))
        .unwrap_or("");
    assert!(
        !native_import_line.contains("Language"),
        "api.py must NOT import Language from the native module; native line: {native_import_line:?}"
    );
    assert!(
        !native_import_line.contains("Parser"),
        "api.py must NOT import Parser from the native module; native line: {native_import_line:?}"
    );

    // Stub assertions: capsule types must not be declared as opaque classes in _native.pyi
    // and function stubs must use `Any` for capsule return types.
    let mut stubs_config = config.clone();
    if let Some(ref mut py) = stubs_config.python {
        py.stubs = Some(alef::core::config::StubsConfig {
            output: std::path::PathBuf::from("packages/python/sample_pack"),
            emit_docstrings: false,
        });
    }
    let stub_files = backend
        .generate_type_stubs(&api, &stubs_config)
        .expect("generate_type_stubs with capsule_types should succeed");
    assert_eq!(stub_files.len(), 1, "expected exactly one .pyi file");
    let stub_content = &stub_files[0].content;

    // Capsule types must NOT appear as standalone class declarations.
    assert!(
        !stub_content.contains("class Language:") && !stub_content.contains("class Language: ..."),
        "stub must NOT declare class Language; content:\n{stub_content}"
    );
    assert!(
        !stub_content.contains("class Parser:") && !stub_content.contains("class Parser: ..."),
        "stub must NOT declare class Parser; content:\n{stub_content}"
    );

    // Free function stubs must return `Any` for capsule types.
    assert!(
        stub_content.contains("def get_language(name: str) -> Any: ..."),
        "stub must contain 'def get_language(name: str) -> Any: ...'; content:\n{stub_content}"
    );
    assert!(
        stub_content.contains("def get_parser(name: str) -> Any: ..."),
        "stub must contain 'def get_parser(name: str) -> Any: ...'; content:\n{stub_content}"
    );

    // The stub must import Any from typing since it is now referenced.
    assert!(
        stub_content.contains("from typing import") && stub_content.contains("Any"),
        "stub must contain 'from typing import ... Any ...'; content:\n{stub_content}"
    );
}

/// capsule_types on impl-block methods:
/// - A type with a method returning a capsule type does NOT produce the non-existent struct.
/// - The method body uses PyCapsule_New (Capsule variant) or Python factory (ConstructFrom).
/// - The generated preamble includes #![allow(unsafe_code)].
#[test]
fn test_capsule_types_in_methods() {
    use alef::core::config::CapsuleTypeConfig;
    use alef::core::ir::{MethodDef, ReceiverKind};

    let backend = Pyo3Backend;

    // IR: an opaque LanguageRegistry type with two methods that return capsule types.
    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            // LanguageRegistry — the opaque registry that owns the Language/Parser getters
            TypeDef {
                name: "LanguageRegistry".to_string(),
                rust_path: "sample_pack::LanguageRegistry".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![
                    // get_language(&self, name: String) -> Result<Language, Error>
                    MethodDef {
                        name: "get_language".to_string(),
                        params: vec![ParamDef {
                            name: "name".to_string(),
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
                        return_type: TypeRef::Named("Language".to_string()),
                        is_async: false,
                        is_static: false,
                        error_type: Some("sample_pack::Error".to_string()),
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
                    },
                ],
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
                doc: "Language registry.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Language — capsule round-trip type (no #[pyclass] emitted)
            TypeDef {
                name: "Language".to_string(),
                rust_path: "sample_pack::Language".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
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

    let mut config = make_config();
    let mut capsule_map: HashMap<String, CapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        CapsuleTypeConfig::Capsule("sample_language.Language".to_string()),
    );
    config.python = Some(PythonConfig {
        module_name: Some("_sample_pack".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: None,
        features: None,
        serde_rename_all: None,
        capsule_types: capsule_map,
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types on methods should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // The #[pymethods] impl block for LanguageRegistry must be present.
    // Regression guard: capsule-method rewriting was stripping the impl block header when the
    // first method returns a capsule type (`attr_start` was incorrectly walking past `#[pymethods]`
    // because `#[pymethods]impl Foo {` starts with `#[`).
    assert!(
        content.contains("#[pymethods]impl LanguageRegistry {")
            || content.contains("#[pymethods]\nimpl LanguageRegistry {"),
        "#[pymethods] impl block opening must be present for LanguageRegistry; content:\n{content}"
    );

    // Language must NOT appear as a standalone #[pyclass] struct — it is a capsule type.
    // Note: "struct LanguageRegistry" is expected; we must not match that as a false positive.
    assert!(
        !content.contains("pub struct Language {") && !content.contains("pub struct Language{"),
        "Language must not be emitted as a #[pyclass] struct; content:\n{content}"
    );

    // The get_language method must use PyCapsule_New (capsule round-trip).
    assert!(
        content.contains("PyCapsule_New"),
        "get_language method must call PyCapsule_New; content:\n{content}"
    );

    // The method must NOT reference the removed Language struct in its return type.
    assert!(
        !content.contains("-> PyResult<Language>"),
        "get_language method must not return PyResult<Language> (struct removed); content:\n{content}"
    );

    // The method must return PyResult<Py<PyAny>> instead.
    assert!(
        content.contains("-> pyo3::PyResult<pyo3::Py<pyo3::PyAny>>"),
        "get_language method must return pyo3::PyResult<pyo3::Py<pyo3::PyAny>>; content:\n{content}"
    );

    // The capsule name constant must be emitted with the configured name.
    assert!(
        content.contains("sample_language.Language"),
        "get_language method must embed the 'sample_language.Language' capsule name; content:\n{content}"
    );

    // The preamble must include #![allow(unsafe_code)].
    assert!(
        content.contains("allow(unsafe_code)"),
        "preamble must include #![allow(unsafe_code)]; content:\n{content}"
    );

    // Stub assertions: LanguageRegistry.get_language must return `Any` in .pyi.
    let mut stubs_config = config.clone();
    if let Some(ref mut py) = stubs_config.python {
        py.stubs = Some(alef::core::config::StubsConfig {
            output: std::path::PathBuf::from("packages/python/sample_pack"),
            emit_docstrings: false,
        });
    }
    let stub_files = backend
        .generate_type_stubs(&api, &stubs_config)
        .expect("generate_type_stubs with capsule_types on methods should succeed");
    assert_eq!(stub_files.len(), 1, "expected exactly one .pyi file");
    let stub_content = &stub_files[0].content;

    // Language must NOT appear as a standalone class declaration.
    assert!(
        !stub_content.contains("class Language:") && !stub_content.contains("class Language: ..."),
        "stub must NOT declare class Language; content:\n{stub_content}"
    );

    // LanguageRegistry must be declared (it is NOT a capsule type).
    assert!(
        stub_content.contains("class LanguageRegistry:"),
        "stub must declare class LanguageRegistry; content:\n{stub_content}"
    );

    // Within LanguageRegistry, get_language must return `Any`.
    assert!(
        stub_content.contains("def get_language(self, name: str) -> Any: ..."),
        "stub must contain 'def get_language(self, name: str) -> Any: ...'; content:\n{stub_content}"
    );

    // Any must be imported.
    assert!(
        stub_content.contains("from typing import") && stub_content.contains("Any"),
        "stub must contain 'from typing import ... Any ...'; content:\n{stub_content}"
    );
}

// ==============================================================================
// Regression tests: UPPER_SNAKE_CASE pyclass enum variants (iter35 wave-1 W2)
// ==============================================================================

fn make_unit_enum_def(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .enumerate()
            .map(|(i, v)| EnumVariant {
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: i == 0,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            })
            .collect(),
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

/// Generated Rust binding emits `#[pyo3(name = "UPPER_SNAKE_CASE")]` on every unit-enum variant
/// when the enum carries the `#[pyclass]` attribute.
#[test]
fn test_pyclass_enum_variants_use_upper_snake_case_pyo3_name() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![make_unit_enum_def(
            "BatchStatus",
            &["Validating", "InProgress", "Complete"],
        )],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let rust_src = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "rs"))
        .map(|f| f.content.as_str())
        .unwrap_or("");

    assert!(
        rust_src.contains("#[pyo3(name = \"VALIDATING\")]"),
        "pyclass enum variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
    assert!(
        rust_src.contains("#[pyo3(name = \"IN_PROGRESS\")]"),
        "multi-word variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
    assert!(
        rust_src.contains("#[pyo3(name = \"COMPLETE\")]"),
        "simple variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
}

/// `options.py` must NOT emit SCREAMING_SNAKE_CASE monkey-patch alias lines for needed unit enums.
/// The canonical UPPER_SNAKE_CASE name is now the direct pyclass variant name, not an alias.
#[test]
fn test_options_py_does_not_emit_screaming_alias_lines() {
    let backend = Pyo3Backend;
    // Use a has_default type that references the enum so it ends up in `needed_enums` and
    // is therefore imported from the native module (the code path that previously monkey-patched).
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("status", TypeRef::Named("BatchStatus".to_string()), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        enums: vec![make_unit_enum_def("BatchStatus", &["Validating", "InProgress"])],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let options_py = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "options.py"))
        .map(|f| f.content.as_str())
        .unwrap_or("");

    // The old pattern was `BatchStatus.VALIDATING = BatchStatus.Validating`
    // or `setattr(BatchStatus, "VALIDATING", getattr(...))`.  Neither should appear.
    assert!(
        !options_py.contains(".VALIDATING = "),
        "options.py must NOT emit SCREAMING alias assignment, got:\n{}",
        options_py
    );
    assert!(
        !options_py.contains("setattr(BatchStatus"),
        "options.py must NOT emit setattr monkey-patch for BatchStatus, got:\n{}",
        options_py
    );
}

/// `options.py` must escape variant names whose snake_case form collides with a Python
/// reserved keyword. The HTML `<del>` tag maps to a Rust `NodeType::Del` variant; without
/// escaping, this emits `del = "del"` which is unparseable as a class-body statement.
/// `alef::core::keywords::python_ident` appends `_` (`del_ = "del"`).
#[test]
fn test_options_py_escapes_python_keyword_variant_names() {
    let backend = Pyo3Backend;
    // Create two enums: one referenced by a has_default type (goes to needed_enums, gets
    // imported) and one unreferenced (emitted as a (str, Enum) class in options.py).
    // We test the unreferenced one to verify the (str, Enum) emission path.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("heading", TypeRef::Named("HeadingStyle".to_string()), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        enums: vec![
            make_unit_enum_def("HeadingStyle", &["Atx", "Setext"]),
            // This enum is not referenced by any has_default type, so it will be emitted
            // as a (str, Enum) class in options.py, allowing us to test the escaping.
            make_unit_enum_def("NodeType", &["Del", "Ins", "Title"]),
        ],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let options_py = files
        .iter()
        .find(|f| f.path.ends_with("options.py"))
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| {
            panic!(
                "options.py not emitted. files: {:?}",
                files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
            )
        });

    assert!(
        options_py.contains("del_ = \"del\"") || options_py.contains("del_ = 'del'"),
        "options.py must escape Python-keyword variant Del → del_ (with original 'del' as value), got:\n{}",
        options_py
    );
    assert!(
        !options_py.contains("\n    del = "),
        "options.py must NOT emit the unescaped keyword `del` as a class attribute, got:\n{}",
        options_py
    );
    assert!(
        options_py.contains("ins = \"ins\"") || options_py.contains("ins = 'ins'"),
        "non-keyword variants must still emit unescaped (ins), got:\n{}",
        options_py
    );
    assert!(
        options_py.contains("title_ = \"title\"") || options_py.contains("title_ = 'title'"),
        "options.py must escape str-method variant Title → title_ (with original 'title' as value), got:\n{}",
        options_py
    );
}

/// Bug A: void-returning functions should NOT emit `return` statement.
/// Functions with `-> None` annotation must emit a bare call without `return`.
#[test]
fn test_api_py_void_function_no_redundant_return() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "init".to_string(),
            rust_path: "test_lib::init".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: "Initialize the system.".to_string(),
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
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not found");

    // The function body should call _rust.init() without a return statement
    assert!(
        api_py.content.contains("def init() -> None:"),
        "api.py should have void-returning init function signature, got:\n{}",
        api_py.content
    );

    // Extract the function body to verify no `return` keyword appears
    if let Some(start) = api_py.content.find("def init() -> None:") {
        // Look for the next function definition to find the end of this function
        let rest = &api_py.content[start..];
        let next_fn_start = rest[19..].find("def ").map(|p| p + 19);
        let fn_body = if let Some(end) = next_fn_start {
            &rest[..end]
        } else {
            rest
        };
        // The body should have the docstring and the call
        assert!(fn_body.contains("_rust.init()"), "Function should call _rust.init()");
        // But it should NOT have "return _rust.init()"
        let without_docstring = fn_body.split("\"\"\"").last().unwrap_or(fn_body);
        assert!(
            !without_docstring.contains("return _rust.init()"),
            "Void-returning function must not emit 'return _rust.init()', got:\n{}",
            fn_body
        );
    }
}

/// Bug B: Consecutive top-level function definitions must have exactly two blank lines between them.
/// This is a PEP 8 requirement for spacing between top-level definitions.
#[test]
fn test_api_py_pep8_blank_lines_between_functions() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "first_function".to_string(),
                rust_path: "test_lib::first_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "First function.".to_string(),
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
                name: "second_function".to_string(),
                rust_path: "test_lib::second_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Second function.".to_string(),
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
                name: "third_function".to_string(),
                rust_path: "test_lib::third_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Third function.".to_string(),
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
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not found");

    // Find the three function definitions and verify spacing
    let first_pos = api_py
        .content
        .find("def first_function")
        .expect("first_function not found");
    let second_pos = api_py
        .content
        .find("def second_function")
        .expect("second_function not found");
    let third_pos = api_py
        .content
        .find("def third_function")
        .expect("third_function not found");

    // Between first and second function
    let between_1_2 = &api_py.content[first_pos..second_pos];
    // Count the blank lines between the functions
    // Should be: closing of first function + empty line + empty line + def of second
    let blank_count_1_2 = between_1_2.matches("\n\n").count();
    assert!(
        blank_count_1_2 >= 1,
        "Between first and second function, should have blank lines, got:\n{}",
        between_1_2
    );

    // Between second and third function
    let between_2_3 = &api_py.content[second_pos..third_pos];
    let blank_count_2_3 = between_2_3.matches("\n\n").count();
    assert!(
        blank_count_2_3 >= 1,
        "Between second and third function, should have blank lines, got:\n{}",
        between_2_3
    );

    // More stringent check: no docstrings immediately followed by def (with only 1 newline).
    // PEP 8 requires 2 blank lines between top-level definitions, meaning 3 newlines total.
    // We check for the docstring closing followed by only 1 or 2 newlines then 'def'.
    let has_improper_spacing_single = api_py.content.contains("\"\"\"\ndef ");
    let has_improper_spacing_one_blank = api_py.content.contains("\"\"\"\n\ndef ");
    assert!(
        !has_improper_spacing_single && !has_improper_spacing_one_blank,
        "Functions are jammed together without proper PEP 8 spacing:\n{}",
        api_py.content
    );
}

/// Regression test: `from ._<module> import (` must NOT be followed by a blank line.
///
/// Previously the multi-line native-import branch routed through `single_line.jinja`
/// with a text ending in `\n`; the template appended a second `\n`, yielding
/// `from ._mod import (\n\n    Name,` which ruff E303 rejects and which caused an
/// endless regen-format-fail loop in downstream consumers (sample_crate, sample_crawler).
#[test]
fn test_native_import_no_stray_blank_line_after_open_paren() {
    let backend = Pyo3Backend;

    // Create enough opaque types (no has_default) so that the import line exceeds
    // 88 chars and the multi-line branch is taken.  Names are intentionally long
    // to force multi-line without needing dozens of types.
    let make_opaque = |name: &str| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            make_opaque("AssetCategory"),
            make_opaque("AuthConfig"),
            make_opaque("BrowserMode"),
            make_opaque("BrowserWait"),
            make_opaque("CrawlEngineHandle"),
            make_opaque("FeedType"),
            make_opaque("ImageSource"),
            make_opaque("LinkType"),
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

    let config = make_config();
    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");

    // The import must start a multi-line block and must NOT have a blank line
    // immediately after the open paren — `(\n\n` is the bug pattern.
    assert!(
        !init_py.content.contains("import (\n\n"),
        "__init__.py must not have a blank line after the open paren in a multi-line import; \
         ruff E303 rejects it and causes an endless regen-format loop.\ncontent:\n{}",
        init_py.content
    );

    // Verify the import block is actually multi-line (the test is only useful
    // if we hit the `import_line.len() > 88` branch).
    assert!(
        init_py.content.contains("from ._test_lib import (\n"),
        "__init__.py should emit a multi-line native import for this many types;\ncontent:\n{}",
        init_py.content
    );
}

/// Adapters (streaming method wrappers):
/// - api.py emits module-level wrapper functions for each adapter
/// - __init__.py imports and re-exports them in __all__
#[test]
fn test_adapter_wrapper_functions() {
    use alef::core::config::{AdapterParam, AdapterPattern};

    let backend = Pyo3Backend;

    // Create a minimal API with a handle type and a function that returns an iterator.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Handle".to_string(),
                rust_path: "test_lib::Handle".to_string(),
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
                doc: "Handle type".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "StreamEvent".to_string(),
                rust_path: "test_lib::StreamEvent".to_string(),
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
                doc: "Stream event type".to_string(),
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

    let mut config = make_config();
    // Add one adapter
    config.adapters = vec![alef::core::config::AdapterConfig {
        name: "test_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "test_stream".to_string(),
        owner_type: Some("Handle".to_string()),
        item_type: Some("StreamEvent".to_string()),
        error_type: None,
        returns: None,
        request_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        params: vec![AdapterParam {
            name: "url".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        skip_languages: vec![],
    }];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    // Check api.py contains the wrapper function
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    // Rust `String` adapter param type must be mapped to Python `str`.
    assert!(
        api_py
            .content
            .contains("async def test_stream(engine: Handle, url: str) -> AsyncIterator[StreamEvent]:"),
        "api.py must map String param to str in streaming wrapper signature; content:\n{}",
        api_py.content
    );

    assert!(
        api_py.content.contains("async for item in engine.test_stream(url):"),
        "api.py must contain async for loop delegating to engine method; content:\n{}",
        api_py.content
    );

    assert!(
        api_py.content.contains("yield item"),
        "api.py must contain yield statement in adapter wrapper; content:\n{}",
        api_py.content
    );

    // Check __init__.py imports and exports the adapter
    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");

    assert!(
        init_py.content.contains("test_stream"),
        "__init__.py must import and export the adapter wrapper; content:\n{}",
        init_py.content
    );

    assert!(
        init_py.content.contains("\"test_stream\"") || init_py.content.contains("'test_stream'"),
        "__init__.py must list test_stream in __all__; content:\n{}",
        init_py.content
    );
}

/// Adapter async_method wrappers:
/// - emit `return await engine.foo(...)` (not `async for item in engine.foo(): yield item`)
/// - return the type from `adapter.returns`
/// - map Rust `String` param type to Python `str`
/// - do NOT add AsyncIterator to the typing imports
#[test]
fn test_async_method_adapter_wrapper() {
    use alef::core::config::{AdapterParam, AdapterPattern};

    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "test_lib::Handle".to_string(),
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
            doc: "Handle type".to_string(),
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

    let mut config = make_config();
    config.adapters = vec![alef::core::config::AdapterConfig {
        name: "fetch_data".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "fetch_data".to_string(),
        owner_type: Some("Handle".to_string()),
        item_type: None,
        returns: Some("DataResult".to_string()),
        error_type: None,
        request_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        params: vec![AdapterParam {
            name: "key".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        skip_languages: vec![],
    }];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    // Must use return-await form, not async-for-yield.
    assert!(
        api_py
            .content
            .contains("async def fetch_data(engine: Handle, key: str) -> DataResult:"),
        "api.py must emit return-await signature for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("return await engine.fetch_data(key)"),
        "api.py must emit `return await engine.fetch_data(key)` for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        !api_py.content.contains("async for item in engine.fetch_data"),
        "api.py must NOT emit async-for loop for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        !api_py.content.contains("AsyncIterator"),
        "api.py must NOT import AsyncIterator when there are no streaming adapters; content:\n{}",
        api_py.content
    );
}

#[test]
fn test_serde_rename_in_constructor_and_properties() {
    let backend = Pyo3Backend;

    // Create a struct with a field that has serde_rename
    let mut field_with_rename = make_field("max_characters", TypeRef::Primitive(PrimitiveType::Usize), true);
    field_with_rename.serde_rename = Some("max_chars".to_string());
    field_with_rename.typed_default = Some(alef::core::ir::DefaultValue::IntLiteral(1000));

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ChunkingConfig".to_string(),
            rust_path: "test_lib::ChunkingConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![field_with_rename, {
                let mut f = make_field("overlap", TypeRef::Primitive(PrimitiveType::Usize), true);
                f.typed_default = Some(alef::core::ir::DefaultValue::IntLiteral(200));
                f
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
            has_serde: true,
            super_traits: vec![],
            doc: "Chunking configuration with serde renames".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    // Find the generated lib.rs
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The PyO3 signature should use max_chars (the serde_rename name)
    assert!(
        lib_rs.content.contains("max_chars=None"),
        "PyO3 signature should use serde_rename 'max_chars=None'; content:\n{}",
        lib_rs.content
    );

    // The constructor parameter should be max_chars
    assert!(
        lib_rs.content.contains("pub fn new(max_chars:"),
        "Constructor parameter should use serde_rename 'max_chars'; content:\n{}",
        lib_rs.content
    );

    // The struct literal should use max_characters (bare Rust field name)
    assert!(
        lib_rs.content.contains("Self { max_characters: max_chars"),
        "Struct literal should use bare field name 'max_characters'; content:\n{}",
        lib_rs.content
    );

    // The serde rename attribute should be present on the field
    assert!(
        lib_rs.content.contains("#[serde(rename = \"max_chars\")]"),
        "Field should have serde(rename = \"max_chars\"); content:\n{}",
        lib_rs.content
    );
}

#[test]
fn test_cfg_gated_fields_excluded_from_constructor() {
    let backend = Pyo3Backend;

    // Create fields: one cfg-gated by a predicate `cfg_present_for_pyo3` cannot prove
    // (a non-feature, non-wasm gate — here a contrived `any(unix, windows)` form),
    // and one ungated. Feature gates (`feature = "pdf"`) are now treated as present
    // because the pyo3 Cargo.toml controls which features compile in; only predicates
    // that may genuinely strip the field at build time are excluded.
    let mut cfg_field = make_field("pdf_options", TypeRef::String, true);
    cfg_field.cfg = Some("any(unix, windows)".to_string());
    cfg_field.typed_default = Some(alef::core::ir::DefaultValue::None);

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                {
                    let mut f = make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false);
                    f.typed_default = Some(alef::core::ir::DefaultValue::BoolLiteral(true));
                    f
                },
                cfg_field,
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: true,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Config with cfg-gated field".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    // Find the generated lib.rs
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The constructor should NOT have pdf_options as a parameter (it's cfg-gated)
    assert!(
        !lib_rs.content.contains("pub fn new(pdf_options:"),
        "Constructor should NOT have cfg-gated parameter 'pdf_options'; content:\n{}",
        lib_rs.content
    );

    // The constructor should have use_cache as a parameter (not cfg-gated)
    assert!(
        lib_rs.content.contains("#[new]\n    pub fn new(use_cache:"),
        "Constructor should have non-cfg parameter 'use_cache'; content:\n{}",
        lib_rs.content
    );

    // The struct literal should include use_cache (shorthand) and pdf_options (explicitly set to None)
    assert!(
        lib_rs.content.contains("Self { use_cache, pdf_options: None }"),
        "Struct literal should use shorthand for non-cfg field and explicit None for cfg-gated optional field; content:\n{}",
        lib_rs.content
    );

    // The pdf_options field should still be in the struct definition
    // (cfg attributes are typically not preserved by PyO3 codegen, but the field itself should be there)
    assert!(
        lib_rs.content.contains("pub pdf_options:"),
        "Field should still exist in struct definition; content:\n{}",
        lib_rs.content
    );
}

/// Regression test: a struct field with `serde(rename = "type")` must generate compilable Rust.
/// Before this fix alef emitted `pub fn new(type: String, ...)` and `Self { item_type: type }` —
/// both invalid because `type` is a Rust keyword.  The fix escapes all Rust keywords in
/// constructor parameters and struct-literal RHS values using raw-identifier syntax (`r#type`).
/// PyO3 strips the `r#` prefix so the Python-facing kwarg name stays `type`.
#[test]
fn test_serde_rename_rust_keyword_emitted_as_raw_ident() {
    let backend = Pyo3Backend;

    // `item_type` field carries serde(rename = "type") — the wire name is a Rust keyword.
    let mut item_type_field = make_field("item_type", TypeRef::String, false);
    item_type_field.serde_rename = Some("type".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ResponseOutputItem".to_string(),
            rust_path: "test_lib::ResponseOutputItem".to_string(),
            original_rust_path: String::new(),
            fields: vec![item_type_field, make_field("content", TypeRef::String, false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "A response output item".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The constructor parameter must use raw-identifier syntax, not the bare keyword `type`.
    assert!(
        lib_rs.content.contains("pub fn new(r#type:"),
        "Constructor parameter for serde-renamed 'type' field must be 'r#type'; content:\n{}",
        lib_rs.content
    );

    // The PyO3 signature attribute must also use the raw identifier (PyO3 strips `r#` → Python sees `type`).
    assert!(
        lib_rs.content.contains("r#type") && !lib_rs.content.contains("(type,") && !lib_rs.content.contains("(type)"),
        "pyo3 signature must not contain bare 'type' token; content:\n{}",
        lib_rs.content
    );

    // The struct literal must assign via raw identifier: `item_type: r#type`.
    assert!(
        lib_rs.content.contains("item_type: r#type"),
        "Struct literal must use 'item_type: r#type' for the renamed field; content:\n{}",
        lib_rs.content
    );

    // Sanity: the field definition should still carry the serde rename attribute.
    assert!(
        lib_rs.content.contains("#[serde(rename = \"type\")]"),
        "Field must retain #[serde(rename = \"type\")] attribute; content:\n{}",
        lib_rs.content
    );
}

/// Regression test: struct fields with `Option<T>` must be emitted as `Option<T>` in constructor
/// signatures, not as bare `T`. This applies to any `T`: `Option<u64>`, `Option<String>`, etc.
#[test]
fn test_option_fields_in_constructor_signature() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OptionalFieldsType".to_string(),
            rust_path: "test_lib::OptionalFieldsType".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("opt_u64", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("opt_string", TypeRef::String, true),
                make_field("opt_duration", TypeRef::Duration, true),
                make_field("required_u32", TypeRef::Primitive(PrimitiveType::U32), false),
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
            doc: "Type with optional fields".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // All optional fields must have Option<T> parameter types, not bare T
    assert!(
        lib_rs.content.contains("pub fn new("),
        "Constructor should exist; content:\n{}",
        lib_rs.content
    );

    // Check for Option<u64> — NOT bare u64
    assert!(
        lib_rs.content.contains("opt_u64: Option<u64>"),
        "Parameter opt_u64 must be Option<u64>, not bare u64; content:\n{}",
        lib_rs.content
    );

    // Check for Option<String> — NOT bare String
    assert!(
        lib_rs.content.contains("opt_string: Option<String>"),
        "Parameter opt_string must be Option<String>, not bare String; content:\n{}",
        lib_rs.content
    );

    // Check for Option<u64> (Duration maps to u64) — NOT bare u64
    assert!(
        lib_rs.content.contains("opt_duration: Option<u64>"),
        "Parameter opt_duration must be Option<u64>, not bare u64; content:\n{}",
        lib_rs.content
    );

    // Required field must be bare type, not optional
    assert!(
        lib_rs.content.contains("required_u32: u32"),
        "Parameter required_u32 must be u32 (not optional); content:\n{}",
        lib_rs.content
    );

    // Defaults should be None for optional fields
    assert!(
        lib_rs.content.contains("opt_u64=None") || lib_rs.content.contains("opt_u64 = None"),
        "Optional field opt_u64 should default to None; content:\n{}",
        lib_rs.content
    );
}

/// Test for Option fields on has_default types (the actual bug case in sample_crawler).
#[test]
fn test_option_fields_on_has_default_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigWithDefaults".to_string(),
            rust_path: "test_lib::ConfigWithDefaults".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("request_timeout", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("name", TypeRef::String, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true, // This is the key difference — has_default
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Config with defaults".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    println!("Generated lib.rs for has_default type:\n{}\n", lib_rs.content);

    // The constructor must have Option<u64> parameters, NOT bare u64
    assert!(
        lib_rs.content.contains("timeout: Option<u64>"),
        "Parameter timeout must be Option<u64> for has_default type; content:\n{}",
        lib_rs.content
    );

    assert!(
        lib_rs.content.contains("request_timeout: Option<u64>"),
        "Parameter request_timeout must be Option<u64> for has_default type; content:\n{}",
        lib_rs.content
    );

    // Defaults should be None for optional fields
    assert!(
        lib_rs.content.contains("timeout=None") || lib_rs.content.contains("timeout = None"),
        "Optional field timeout should default to None; content:\n{}",
        lib_rs.content
    );
}

/// Test for Option fields on has_default types WITH serde_rename.
#[test]
fn test_option_fields_with_serde_rename_on_has_default() {
    let backend = Pyo3Backend;

    let mut timeout_field = make_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true);
    timeout_field.serde_rename = Some("timeout_ms".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "RequestOptions".to_string(),
            rust_path: "test_lib::RequestOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![timeout_field, make_field("name", TypeRef::String, false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Request options".to_string(),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    println!(
        "Generated lib.rs for has_default type with serde_rename:\n{}\n",
        lib_rs.content
    );

    // The constructor parameter for serde-renamed optional field must still be Option<u64>
    assert!(
        lib_rs.content.contains("timeout_ms: Option<u64>"),
        "Parameter timeout_ms must be Option<u64> even with serde_rename; content:\n{}",
        lib_rs.content
    );

    // Verify it defaults to None
    assert!(
        lib_rs.content.contains("timeout_ms=None") || lib_rs.content.contains("timeout_ms = None"),
        "Optional field timeout_ms should default to None; content:\n{}",
        lib_rs.content
    );
}

#[test]
fn test_has_default_struct_with_nested_struct_field_accepts_none() {
    // This test verifies BLK-5 fix: a has_default struct with a non-optional
    // nested-struct field whose type also has has_default=true should accept None
    // in the constructor, with an unwrap_or_else falling back to the nested type's default.
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            // Nested struct that derives Default
            TypeDef {
                name: "PreprocessingOptions".to_string(),
                rust_path: "test_lib::PreprocessingOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("normalize", TypeRef::Primitive(PrimitiveType::Bool), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true, // This type derives Default
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Preprocessing options".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Parent struct with has_default=true owning non-optional PreprocessingOptions
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    // This is the critical case: non-optional nested struct field on a has_default type
                    // Should be emitted as Option<PreprocessingOptions> with default None
                    make_field(
                        "preprocessing",
                        TypeRef::Named("PreprocessingOptions".to_string()),
                        false,
                    ),
                    make_field("format", TypeRef::String, false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true, // Parent also derives Default
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Conversion options".to_string(),
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

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings: {}", result.unwrap_err());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify that the ParseOptions constructor parameter 'preprocessing' is Option<PreprocessingOptions>
    // The parameter should be declared as Option<PreprocessingOptions>
    assert!(
        content.contains("preprocessing: Option<PreprocessingOptions>"),
        "Parameter 'preprocessing' must be Option<PreprocessingOptions> to accept None; content:\n{}",
        content
    );

    // Verify the default is None
    assert!(
        content.contains("preprocessing=None") || content.contains("preprocessing = None"),
        "Parameter 'preprocessing' should default to None; content:\n{}",
        content
    );

    // Verify the assignment uses unwrap_or_else to fall back to the nested type's default
    assert!(
        content.contains("preprocessing.unwrap_or_else(|| Self::default().preprocessing)"),
        "Assignment must use unwrap_or_else fallback; content:\n{}",
        content
    );
}

#[test]
fn test_options_field_bridge_field_not_duplicated_when_cfg_force_restored() {
    // Regression test: when a trait-bridge `bind_via = OptionsField` field is also
    // cfg-gated on a `has_default` type, the backend force-restores it into
    // `never_skip_cfg_field_names`. The constructor rewriter must filter it out of
    // `sorted_fields` (so it does not appear via the params iterator) and rely on
    // the existing `bridge_param` append at the end of the param list — otherwise
    // the field appears twice and rustc rejects with E0415
    // ("identifier 'visitor' is bound more than once in this parameter list").
    let backend = Pyo3Backend;

    let mut visitor_field = make_field(
        "visitor",
        TypeRef::Optional(Box::new(TypeRef::Named("VisitorHandle".to_string()))),
        true,
    );
    visitor_field.cfg = Some("feature = \"visitor\"".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "VisitorHandle".to_string(),
                rust_path: "test_lib::VisitorHandle".to_string(),
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
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("format", TypeRef::String, false), visitor_field],
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

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "SyntaxWalker".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: alef::core::config::BridgeBinding::OptionsField,
        options_type: Some("ParseOptions".to_string()),
        options_field: Some("visitor".to_string()),
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }];

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings: {}", result.unwrap_err());

    let files = result.unwrap();
    let content = &files[0].content;

    let conversion_options_block = content
        .split("impl ParseOptions")
        .nth(1)
        .expect("ParseOptions impl block must exist");
    let constructor_body = conversion_options_block
        .split("pub fn new(")
        .nth(1)
        .and_then(|s| s.split(") -> Self").next())
        .expect("ParseOptions::new param list must exist");

    let visitor_param_count = constructor_body.matches("visitor:").count();
    assert_eq!(
        visitor_param_count, 1,
        "ParseOptions::new must declare `visitor:` exactly once, found {} in:\n{}",
        visitor_param_count, constructor_body
    );
}
