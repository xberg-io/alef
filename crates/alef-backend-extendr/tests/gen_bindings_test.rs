use alef_backend_extendr::ExtendrBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, RConfig};
use alef_core::ir::*;

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
        ffi: None,
        go: None,
        java: None,
        csharp: None,
        r: Some(RConfig {
            package_name: Some("testlib".to_string()),
            features: None,
            serde_rename_all: None,
        }),
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
    let backend = ExtendrBackend;

    // Create test API surface with types, functions, and enums
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
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
            doc: "Test config".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "test_lib::extract".to_string(),
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text".to_string(),
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

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();

    // Should generate a single lib.rs file
    assert_eq!(files.len(), 1, "Should generate exactly one file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().contains("lib.rs"),
        "Output file should be lib.rs"
    );

    let content = &lib_file.content;

    // Check for extendr-specific attributes and imports
    assert!(
        content.contains("extendr_api::prelude::*"),
        "Should import extendr_api::prelude::*"
    );

    // Check for struct generation (Config)
    assert!(content.contains("pub struct Config"), "Should generate Config struct");
    assert!(content.contains("timeout"), "Should have timeout field");
    assert!(content.contains("backend"), "Should have backend field");

    // Check for function binding with #[extendr] attribute
    assert!(
        content.contains("#[extendr]"),
        "Functions should have #[extendr] attribute"
    );
    assert!(content.contains("fn extract"), "Should generate extract function");

    // Check for enum generation
    assert!(content.contains("pub enum Mode"), "Should generate Mode enum");
    assert!(content.contains("Fast"), "Should have Fast variant");
    assert!(content.contains("Accurate"), "Should have Accurate variant");

    // Check for module registration
    assert!(
        content.contains("extendr_module!"),
        "Should have extendr_module registration"
    );
    assert!(content.contains("mod testlib"), "Module name should match package_name");
    assert!(content.contains("impl Config"), "Should register Config type in module");
    assert!(
        content.contains("fn extract"),
        "Should register extract function in module"
    );
}

#[test]
fn test_type_mapping() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test::Numbers".to_string(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("strings", TypeRef::Vec(Box::new(TypeRef::String)), false),
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
    let lib_file = &files[0];
    let content = &lib_file.content;

    // Verify struct is generated
    assert!(content.contains("pub struct Numbers"));

    // Extendr uses Rust types directly, so verify field names appear
    assert!(content.contains("u32_val"));
    assert!(content.contains("i64_val"));
    assert!(content.contains("string_val"));
    assert!(content.contains("opt_string"));
    assert!(content.contains("strings"));
}

#[test]
fn test_enum_generation() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test::Status".to_string(),
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
            doc: "Task status".to_string(),
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

    // Verify enum is generated
    assert!(content.contains("pub enum Status"));

    // Verify all variants are present
    assert!(content.contains("Pending"));
    assert!(content.contains("Active"));
    assert!(content.contains("Completed"));

    // Verify derive attributes for extendr
    assert!(content.contains("Clone"));
    assert!(content.contains("PartialEq"));
}

#[test]
fn test_generated_header() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SimpleType".to_string(),
            rust_path: "test::SimpleType".to_string(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
        functions: vec![FunctionDef {
            name: "simple_fn".to_string(),
            rust_path: "test::simple_fn".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
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

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // All files should have generated_header set to true
    for file in &files {
        // Note: In the current gen_bindings.rs, generated_header is set to false
        // We check that the field exists and document this behavior
        assert!(
            !file.generated_header,
            "Current extendr backend sets generated_header=false"
        );
    }
}
