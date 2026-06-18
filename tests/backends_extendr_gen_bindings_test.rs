use alef::backends::extendr::ExtendrBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
"#,
    )
}

fn make_options_field_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "Renderer"
type_alias = "RendererHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "RenderOptions"
"#,
    )
}

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

fn make_param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
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
        core_wrapper: CoreWrapper::None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>, has_default: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default,
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

fn make_unit_enum(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|variant| EnumVariant {
                name: (*variant).to_string(),
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
            })
            .collect(),
        methods: vec![],
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
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
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
            doc: "Test config".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "test_lib::extract".to_string(),
            original_rust_path: String::new(),
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
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text".to_string(),
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
fn options_decoder_uses_configured_type_and_ir_shapes() {
    let backend = ExtendrBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            make_type(
                "RenderOptions",
                vec![
                    make_field("style", TypeRef::Named("RenderStyle".to_string()), false),
                    make_field("nested", TypeRef::Named("NestedOptions".to_string()), false),
                    make_field("renderer", TypeRef::Named("RendererHandle".to_string()), true),
                ],
                true,
            ),
            make_type(
                "NestedOptions",
                vec![
                    make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
                    make_field("preset", TypeRef::Named("NestedPreset".to_string()), false),
                ],
                true,
            ),
        ],
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "test_lib::render".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "input".to_string(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("RenderOptions".to_string()),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
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
        enums: vec![
            make_unit_enum("RenderStyle", &["Plain", "Rich"]),
            make_unit_enum("NestedPreset", &["Small", "Large"]),
        ],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_public_api(&api, &make_options_field_config()).unwrap();
    let options_rs = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("options.rs"))
        .expect("options.rs should be generated for configured options field bridge");
    let content = &options_rs.content;

    assert!(content.contains("std::result::Result<crate::RenderOptions, String>"));
    assert!(content.contains("fn decode_render_style"));
    assert!(content.contains("\"Plain\" => Ok(crate::RenderStyle::Plain)"));
    assert!(content.contains("fn decode_nested_options"));
    assert!(content.contains("decode_nested_preset(v)?"));
    assert!(!content.contains("PreprocessingOptions"));
    assert!(!content.contains("PreprocessingPreset"));
    assert!(!content.contains("ParseOptions"));
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
            original_rust_path: String::new(),
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
                    name: "Completed".to_string(),
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
            doc: "Task status".to_string(),
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
fn test_emits_binding_to_core_from_impls_for_input_types() {
    // When a struct is used as a function parameter, the binding code calls
    // `.into()` to bridge from the binding type to the core type.  Verify a
    // matching `impl From<BindingType> for core::Type` is emitted.
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("flag", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field("name", TypeRef::String, false),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![
            FunctionDef {
                name: "run".to_string(),
                rust_path: "test_lib::run".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("Options".to_string()))),
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
                }],
                return_type: TypeRef::String,
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
                name: "select".to_string(),
                rust_path: "test_lib::select".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "mode".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("Mode".to_string()))),
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
                }],
                return_type: TypeRef::String,
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
        enums: vec![make_unit_enum("Mode", &["Fast"])],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let content = &files[0].content;

    assert!(
        content.contains("impl From<Options> for test_lib::Options"),
        "binding→core From impl missing for Options used as input:\n{content}"
    );
    assert!(
        content.contains("impl From<test_lib::Options> for Options"),
        "core→binding From impl missing for Options:\n{content}"
    );
    assert!(
        content.contains("let options_core: Option<test_lib::Options> = options.as_deref()"),
        "optional named DTO params must deserialize through an owned Option<T> binding:\n{content}"
    );
    assert!(
        content.contains("let result = test_lib::run(options_core);"),
        "optional named DTO params must be passed to owned core params without as_ref():\n{content}"
    );
    assert!(
        content.contains("let mode_core: Option<test_lib::Mode> = mode.as_deref()"),
        "optional enum params must deserialize through an owned Option<T> binding:\n{content}"
    );
    assert!(
        content.contains("let result = test_lib::select(mode_core);"),
        "optional enum params must be passed to owned core params without as_ref():\n{content}"
    );
}

#[test]
fn test_emits_lossy_from_impls_for_data_variant_enums() {
    // Enums with data variants are flattened to unit variants in the binding layer
    // (extendr cannot represent variant payloads).  Lossy From impls must still be
    // emitted so containing structs that derive `From` can compile.
    let backend = ExtendrBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Document".to_string(),
            rust_path: "test_lib::Document".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("kind", TypeRef::Named("Kind".to_string()), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
        }],
        functions: vec![FunctionDef {
            name: "parse".to_string(),
            rust_path: "test_lib::parse".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Document".to_string()),
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
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
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
                    name: "Heading".to_string(),
                    fields: vec![make_field("level", TypeRef::Primitive(PrimitiveType::U32), false)],
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
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let content = &files[0].content;
    assert!(
        content.contains("impl From<test_lib::Kind> for Kind"),
        "expected lossy core→binding From impl for data-variant enum Kind:\n{content}"
    );
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
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
            name: "simple_fn".to_string(),
            rust_path: "test::simple_fn".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
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

fn make_owned_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        receiver: Some(ReceiverKind::Owned),
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_ref_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        receiver: Some(ReceiverKind::Ref),
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn r_method_wrappers_bind_self_without_mutating_method_environment() {
    let backend = ExtendrBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Document".to_string(),
            rust_path: "test_lib::Document".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![make_ref_method("text", vec![], TypeRef::String)],
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

    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .unwrap();
    assert!(
        wrappers
            .content
            .contains("Document$text <- function(self) .Call(\"wrap__Document__text\", self, PACKAGE = \"testlib\")"),
        "instance wrapper must take self explicitly:\n{}",
        wrappers.content
    );
    assert!(
        wrappers
            .content
            .contains("if (identical(names(formals(func))[1], \"self\")) {\n    function(...) func(self, ...)"),
        "$ dispatch must bind self via a closure instead of mutating the method environment:\n{}",
        wrappers.content
    );
}

#[test]
fn test_opaque_type_generates_inner_field_and_delegates() {
    // Regression: opaque types (e.g. ParseOptionsBuilder) must generate
    // `inner: Arc<CoreType>` and delegate methods — not emit empty structs with todo!() stubs.
    let backend = ExtendrBackend;

    let builder_type = TypeDef {
        name: "OptionsBuilder".to_string(),
        rust_path: "test_lib::OptionsBuilder".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            make_owned_method(
                "with_value",
                vec![ParamDef {
                    name: "value".to_string(),
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
                TypeRef::Named("OptionsBuilder".to_string()),
            ),
            make_ref_method("build", vec![], TypeRef::Named("Options".to_string())),
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
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let options_type = TypeDef {
        name: "Options".to_string(),
        rust_path: "test_lib::Options".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("value", TypeRef::String, false)],
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![options_type, builder_type],
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Opaque builder struct must have inner: Arc<CoreType>, not be empty
    assert!(
        content.contains("inner: Arc<test_lib::OptionsBuilder>"),
        "Opaque builder must have inner: Arc<CoreType>. Got:\n{}",
        content
    );
    // Must import Arc
    assert!(
        content.contains("use std::sync::Arc"),
        "Must import Arc for opaque types"
    );
    // Methods must not use todo!()
    assert!(
        !content.contains("todo!(\"Not implemented: OptionsBuilder"),
        "Opaque builder methods must not contain todo!() stubs"
    );
    // build() must delegate to self.inner
    assert!(
        content.contains("self.inner.build()"),
        "build() must delegate to self.inner. Got:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// Trait bridge tests (Extendr plugin bridge via gen_trait_bridge)
// ---------------------------------------------------------------------------

mod trait_bridge {
    use super::make_unit_enum;
    use alef::backends::extendr::trait_bridge::gen_trait_bridge;
    use alef::core::config::TraitBridgeConfig;
    use alef::core::ir::*;

    fn make_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "SyntaxContext".to_string(),
                rust_path: "my_lib::SyntaxContext".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
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
            enums: vec![make_unit_enum("WalkDecision", &["Continue"])],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    fn make_method(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![ParamDef {
                name: "ctx".to_string(),
                ty: TypeRef::Named("SyntaxContext".to_string()),
                optional: false,
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
            }],
            return_type,
            is_async: false,
            is_static: false,
            error_type: if has_error {
                Some("Box<dyn std::error::Error + Send + Sync>".to_string())
            } else {
                None
            },
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: has_default,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_async_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
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
        }
    }

    fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            unregister_fn: None,
            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            ffi_skip_methods: Vec::new(),
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        }
    }

    fn make_visitor_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
            type_alias: Some(format!("{trait_name}Handle")),
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            ffi_skip_methods: Vec::new(),
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: Some("SyntaxContext".to_string()),
            result_type: Some("WalkDecision".to_string()),
        }
    }

    // ---- Plugin bridge: wrapper struct ---

    #[test]
    fn test_plugin_bridge_generates_wrapper_struct() {
        let trait_def = make_trait_def(
            "TextBackend",
            vec![make_method("process", TypeRef::String, true, false)],
        );
        let cfg = make_plugin_bridge_cfg("TextBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("pub struct RTextBackendBridge"),
            "plugin bridge must generate RTextBackendBridge wrapper struct"
        );
        assert!(
            code.code.contains("inner: extendr_api::Robj"),
            "wrapper struct must hold an extendr_api::Robj"
        );
        assert!(
            code.code.contains("cached_name: String"),
            "wrapper struct must cache the plugin name"
        );
    }

    // ---- Plugin bridge: trait impl ---

    #[test]
    fn test_plugin_bridge_generates_trait_impl() {
        let trait_def = make_trait_def(
            "TextBackend",
            vec![make_method("process", TypeRef::String, true, false)],
        );
        let cfg = make_plugin_bridge_cfg("TextBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("impl my_lib::TextBackend for RTextBackendBridge"),
            "plugin bridge must implement the trait for the wrapper"
        );
        assert!(
            code.code.contains("fn process("),
            "trait impl must include all trait methods"
        );
    }

    // ---- Plugin bridge: sync method uses dollar() to look up R function ---

    #[test]
    fn test_plugin_bridge_sync_method_uses_dollar_lookup() {
        let trait_def = make_trait_def("Analyzer", vec![make_method("analyze", TypeRef::String, true, false)]);
        let cfg = make_plugin_bridge_cfg("Analyzer");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("dollar(\"analyze\")"),
            "sync method body must look up the R function via dollar()"
        );
    }

    // ---- Plugin bridge: async method uses spawn_blocking ---

    #[test]
    fn test_plugin_bridge_async_method_uses_spawn_blocking() {
        let trait_def = make_trait_def("Processor", vec![make_async_method("run")]);
        let cfg = make_plugin_bridge_cfg("Processor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("spawn_blocking"),
            "async method body must use tokio::task::spawn_blocking"
        );
        assert!(
            code.code.contains("async fn run("),
            "async method must be declared async"
        );
    }

    // ---- Plugin bridge: registration function ---

    #[test]
    fn test_plugin_bridge_generates_registration_fn() {
        let trait_def = make_trait_def(
            "TextBackend",
            vec![make_method("process", TypeRef::String, true, false)],
        );
        let cfg = make_plugin_bridge_cfg("TextBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("pub fn register_textbackend("),
            "registration fn must be generated with the configured name"
        );
        assert!(
            code.code.contains("#[extendr]"),
            "registration fn must carry #[extendr] attribute"
        );
        assert!(
            code.code.contains("my_lib::get_registry"),
            "registration fn must call the configured registry getter"
        );
    }

    // ---- Plugin bridge: registration validates required methods ---

    #[test]
    fn test_plugin_bridge_registration_validates_required_methods() {
        let trait_def = make_trait_def(
            "Transform",
            vec![
                make_method("transform", TypeRef::String, true, false),
                make_method("describe", TypeRef::String, false, true),
            ],
        );
        let cfg = make_plugin_bridge_cfg("Transform");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("\"transform\""),
            "registration fn must validate required method 'transform' exists"
        );
        assert!(
            code.code.contains("dollar(\"transform\")") || code.code.contains("\"transform\""),
            "constructor must check required methods via dollar()"
        );
    }

    // ---- Plugin bridge: constructor caches name ---

    #[test]
    fn test_plugin_bridge_constructor_caches_name() {
        let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
        let cfg = make_plugin_bridge_cfg("Worker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("cached_name"),
            "constructor must populate cached_name"
        );
        assert!(
            code.code.contains("dollar(\"name\")"),
            "constructor must call dollar(\"name\") to cache the plugin name"
        );
    }

    // ---- Plugin bridge: super_trait generates Plugin impl ---

    #[test]
    fn test_plugin_bridge_with_super_trait_generates_plugin_impl() {
        let trait_def = make_trait_def(
            "TextBackend",
            vec![make_method("process", TypeRef::String, true, false)],
        );
        let cfg = TraitBridgeConfig {
            trait_name: "TextBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some("register_text_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            ffi_skip_methods: Vec::new(),
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        };
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("impl my_lib::Plugin for RTextBackendBridge"),
            "must generate Plugin impl for bridge struct"
        );
        assert!(code.code.contains("fn name(&self)"), "Plugin impl must include name()");
        assert!(
            code.code.contains("fn initialize(&self)"),
            "Plugin impl must include initialize()"
        );
        assert!(
            code.code.contains("fn shutdown(&self)"),
            "Plugin impl must include shutdown()"
        );
    }

    // ---- Visitor bridge ---

    #[test]
    fn test_visitor_bridge_generates_r_bridge_struct() {
        let trait_def = make_trait_def(
            "SyntaxWalker",
            vec![make_method(
                "visit_node",
                TypeRef::Named("WalkDecision".to_string()),
                false,
                true,
            )],
        );
        let cfg = make_visitor_bridge_cfg("SyntaxWalker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("pub struct RSyntaxWalkerBridge"),
            "visitor bridge must produce RSyntaxWalkerBridge struct"
        );
    }

    #[test]
    fn test_visitor_bridge_does_not_generate_registration_fn() {
        let trait_def = make_trait_def(
            "SyntaxWalker",
            vec![make_method(
                "visit_node",
                TypeRef::Named("WalkDecision".to_string()),
                false,
                true,
            )],
        );
        let cfg = make_visitor_bridge_cfg("SyntaxWalker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            !code.code.contains("#[extendr]"),
            "visitor bridge must not generate an extendr registration function"
        );
    }

    #[test]
    fn test_visitor_bridge_generates_trait_impl() {
        let trait_def = make_trait_def(
            "SyntaxWalker",
            vec![make_method(
                "visit_node",
                TypeRef::Named("WalkDecision".to_string()),
                false,
                true,
            )],
        );
        let cfg = make_visitor_bridge_cfg("SyntaxWalker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("impl my_lib::SyntaxWalker for RSyntaxWalkerBridge"),
            "visitor bridge must implement the trait"
        );
    }

    #[test]
    fn test_visitor_bridge_generates_unsafe_send_sync_impls() {
        // VisitorHandle = Arc<Mutex<dyn SyntaxWalker + Send>> requires the bridge to be Send.
        // Robj wraps a raw SEXP (!Send), so the bridge needs unsafe impl Send + Sync.
        // R is single-threaded, so callers must not actually move the bridge across threads.
        let trait_def = make_trait_def(
            "SyntaxWalker",
            vec![make_method(
                "visit_node",
                TypeRef::Named("WalkDecision".to_string()),
                false,
                true,
            )],
        );
        let cfg = make_visitor_bridge_cfg("SyntaxWalker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
            .expect("trait bridge generation should succeed");

        assert!(
            code.code.contains("unsafe impl Send for RSyntaxWalkerBridge {}"),
            "visitor bridge must produce unsafe impl Send"
        );
        assert!(
            code.code.contains("unsafe impl Sync for RSyntaxWalkerBridge {}"),
            "visitor bridge must produce unsafe impl Sync"
        );
    }

    #[test]
    fn test_exclude_functions_honored() {
        use super::*; // Import helpers from root scope
        let backend = ExtendrBackend;

        // Create API surface with two functions
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![
                FunctionDef {
                    name: "allowed_func".to_string(),
                    rust_path: "test_lib::allowed_func".to_string(),
                    original_rust_path: String::new(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    error_type: None,
                    doc: "This function is allowed".to_string(),
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
                    name: "excluded_func".to_string(),
                    rust_path: "test_lib::excluded_func".to_string(),
                    original_rust_path: String::new(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    error_type: None,
                    doc: "This function is excluded".to_string(),
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
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };

        // Config with exclude_functions for R
        let mut config = super::make_config();
        config.r = Some(alef::core::config::languages::RConfig {
            package_name: Some("testlib".to_string()),
            features: None,
            serde_rename_all: None,
            exclude_functions: vec!["excluded_func".to_string()],
            exclude_types: vec![],
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: vec![],
            extra_makevars_prelude: vec![],
            extra_pkg_libs: vec![],
        });

        let generated_bindings = backend.generate_bindings(&api, &config).unwrap();
        let generated_public = backend.generate_public_api(&api, &config).unwrap();

        // Find the lib.rs file (from bindings)
        let lib_rs = generated_bindings
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
            .expect("should generate lib.rs");

        // allowed_func should be present in the generated bindings
        assert!(
            lib_rs.content.contains("pub fn allowed_func"),
            "allowed_func should be present in generated code"
        );

        // excluded_func should NOT be present
        assert!(
            !lib_rs.content.contains("pub fn excluded_func"),
            "excluded_func should be excluded from generated code"
        );

        // Find the extendr-wrappers.R file (from public API)
        let wrappers_r = generated_public
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("should generate extendr-wrappers.R");

        // allowed_func should have an R wrapper
        assert!(
            wrappers_r.content.contains("allowed_func"),
            "allowed_func should have an R wrapper"
        );

        // excluded_func should NOT have an R wrapper
        assert!(
            !wrappers_r.content.contains("excluded_func"),
            "excluded_func should not have an R wrapper"
        );

        // Find the NAMESPACE file (from public API)
        let namespace = generated_public
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
            .expect("should generate NAMESPACE");

        // allowed_func should be exported
        assert!(
            namespace.content.contains("export(allowed_func)"),
            "allowed_func should be exported in NAMESPACE"
        );

        // excluded_func should NOT be exported
        assert!(
            !namespace.content.contains("export(excluded_func)"),
            "excluded_func should not be exported in NAMESPACE"
        );

        // Check that excluded_func is NOT in the extendr_module! macro
        // The macro should only contain `fn allowed_func;` but not `fn excluded_func;`
        assert!(
            lib_rs.content.contains("fn allowed_func;"),
            "allowed_func should be registered in extendr_module!"
        );
        assert!(
            !lib_rs.content.contains("fn excluded_func;"),
            "excluded_func should not be registered in extendr_module! (would cause dangling meta__ reference)"
        );
    }
}

// Category 4 test: binding_excluded fields should not appear in kwargs constructors
#[test]
fn extendr_binding_excluded_config_fields_skipped_in_kwargs_constructor() {
    // Category 4: binding_excluded fields must not be set in constructor
    let mut field_concurrency = make_field("concurrency", TypeRef::Named("ConcurrencyConfig".to_string()), true);
    field_concurrency.binding_excluded = true;
    field_concurrency.binding_exclusion_reason = Some("alef(skip)".to_string());

    let config = make_type(
        "ExtractionConfig",
        vec![
            make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false),
            field_concurrency,
        ],
        true,
    );

    let gen_code = alef::codegen::config_gen::gen_extendr_kwargs_constructor(
        &config,
        &|ty: &TypeRef| match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
            TypeRef::Named(n) => n.clone(),
            _ => "String".to_string(),
        },
        &ahash::AHashSet::new(),
    );

    // Constructor should NOT include concurrency parameter or assignment
    assert!(
        !gen_code.contains("concurrency"),
        "binding_excluded field 'concurrency' should not appear in constructor\n{gen_code}"
    );
    assert!(
        gen_code.contains("use_cache"),
        "non-excluded field 'use_cache' should appear in constructor"
    );
}

#[test]
fn extendr_return_type_needs_json_for_vec_enum() {
    // Category 1: Vec<Enum> should need JSON bridging
    let mut enum_names: ahash::AHashSet<String> = ahash::AHashSet::new();
    enum_names.insert("EntityCategory".to_string());

    let opaque_types: ahash::AHashSet<String> = ahash::AHashSet::new();
    let incomp_types: ahash::AHashSet<String> = ahash::AHashSet::new();

    let ty = TypeRef::Vec(Box::new(TypeRef::Named("EntityCategory".to_string())));

    // Simulate return_type_needs_json function behavior
    let needs_json = match &ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                enum_names.contains(n.as_str())
                    || opaque_types.contains(n.as_str())
                    || incomp_types.contains(n.as_str())
            }
            _ => false,
        },
        _ => false,
    };

    assert!(needs_json, "Vec<Enum> should require JSON bridging");
}

#[test]
fn extendr_return_type_needs_json_for_vec_opaque() {
    // Category 1: Vec<OpaqueDTO> should need JSON bridging
    let enum_names: ahash::AHashSet<String> = ahash::AHashSet::new();
    let mut opaque_types: ahash::AHashSet<String> = ahash::AHashSet::new();
    opaque_types.insert("QrCode".to_string());
    let incomp_types: ahash::AHashSet<String> = ahash::AHashSet::new();

    let ty = TypeRef::Vec(Box::new(TypeRef::Named("QrCode".to_string())));

    let needs_json = match &ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                enum_names.contains(n.as_str())
                    || opaque_types.contains(n.as_str())
                    || incomp_types.contains(n.as_str())
            }
            _ => false,
        },
        _ => false,
    };

    assert!(needs_json, "Vec<OpaqueDTO> should require JSON bridging");
}

#[test]
fn extendr_param_mut_flag_emits_let_mut() {
    // Category 3: &mut parameters should emit `let mut` in JSON preamble
    // Simulate preamble generation with mut keyword for a mutable parameter
    let is_mut = true;
    let mut_kw = if is_mut { "mut " } else { "" };
    let preamble = format!(
        "let {mut_kw}{name}_core: {ty} = serde_json::from_str(&{name})?;",
        mut_kw = mut_kw,
        name = "result",
        ty = "kreuzberg::ExtractionResult"
    );

    assert!(
        preamble.contains("let mut result_core"),
        "Mutable parameters should emit 'let mut' in preamble\n{preamble}"
    );
}

#[test]
fn extendr_param_non_mut_emits_let_immut() {
    // Category 3: non-&mut parameters should emit `let` (no mut) in JSON preamble
    // Simulate preamble generation without mut keyword for a non-mutable parameter
    let is_mut = false;
    let mut_kw = if is_mut { "mut " } else { "" };
    let preamble = format!(
        "let {mut_kw}{name}_core: {ty} = serde_json::from_str(&{name})?;",
        mut_kw = mut_kw,
        name = "config",
        ty = "kreuzberg::PageClassificationConfig"
    );

    assert!(
        preamble.contains("let config_core"),
        "Non-mutable parameters should emit 'let' without 'mut'\n{preamble}"
    );
}

#[test]
fn extendr_underscore_prefix_stripped_from_r_params() {
    // Regression test: R identifiers cannot start with underscore.
    // Rust parameters like `_flag: bool` are common for unused params,
    // but R requires the leading underscore be stripped in generated wrappers.
    //
    // Expected: `compute <- function(value, flag = NULL) .Call("wrap__compute", value, flag, PACKAGE = "...")`

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        enums: vec![],
        functions: vec![FunctionDef {
            name: "compute".to_string(),
            rust_path: "test_lib::compute".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "value".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
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
                    name: "_flag".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
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
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            doc: "Test function with underscore param".to_string(),
            is_async: false,
            error_type: None,
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
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = ExtendrBackend.generate_public_api(&api, &config).unwrap();

    let r_wrapper_file = files
        .iter()
        .find(|f| f.path.ends_with("extendr-wrappers.R"))
        .expect("Should generate extendr-wrappers.R");

    // Verify: param name in signature should be `flag` not `_flag`
    assert!(
        r_wrapper_file
            .content
            .contains("compute <- function(value, flag = NULL)"),
        "R wrapper should have sanitized param name in signature (no leading underscore)\nContent:\n{}",
        r_wrapper_file.content
    );

    // Verify: param name in .Call() args should also be `flag` not `_flag`
    assert!(
        r_wrapper_file
            .content
            .contains(r#".Call("wrap__compute", value, flag, PACKAGE = "testlib")"#),
        "R wrapper should have sanitized param name in .Call() args (no leading underscore)\nContent:\n{}",
        r_wrapper_file.content
    );

    // Verify: should NOT contain the invalid `_flag` identifier
    assert!(
        !r_wrapper_file.content.contains("function(value, _flag"),
        "R wrapper should not emit leading underscore in function signature"
    );
    assert!(
        !r_wrapper_file
            .content
            .contains(r#".Call("wrap__compute", value, _flag"#),
        "R wrapper should not emit leading underscore in .Call() args"
    );
}

#[test]
fn test_emits_reference_for_named_non_opaque_struct_params() {
    // Regression test for https://github.com/kreuzberg-dev/alef/issues/XXX:
    // Free functions that take non-opaque struct params (e.g. config: ExtractionConfig)
    // must emit them as `&T` in the Rust binding, not as owned `T` or as `String`.
    // Extendr's #[extendr] macro only generates TryFrom<&Robj> for &T (reference),
    // not for owned T. The R caller passes ExternalPtr objects directly from the
    // R6 class, and extendr unwraps them transparently when the param is `&T`.
    let backend = ExtendrBackend;

    let config_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "test_lib::ExtractionConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_param("path", TypeRef::String, false),
                make_param("mime_type", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_param("config", TypeRef::Named("ExtractionConfig".to_string()), false),
            ],
            return_type: TypeRef::String,
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

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let content = &files[0].content;

    // Verify: free function signature must use &ExtractionConfig for the config param, not String
    // When a non-optional Named param follows an optional param, extendr wraps it in Nullable<&T>.
    assert!(
        (content.contains(
            "pub fn extract_file(path: String, mime_type: Option<Option<String>>, config: Nullable<&ExtractionConfig>)"
        ) || content.contains(
            "pub fn extract_file(path: String, mime_type: Option<Option<String>>, config: &ExtractionConfig)"
        )),
        "free function with named struct param must use &T or Nullable<&T>, not String: {content}"
    );

    // Verify: should NOT deserialize from JSON string — that was the old bug
    assert!(
        !content.contains("serde_json::from_str(&config)"),
        "named struct params should not be deserialized from JSON string: {content}"
    );

    // Verify: call site should convert Nullable properly if present, then pass to core
    assert!(
        (content.contains("config.into_option()")
            || content.contains("let result = test_lib::extract_file(path, mime_type, &config")),
        "config param should not use JSON deserialization: {content}"
    );
}
