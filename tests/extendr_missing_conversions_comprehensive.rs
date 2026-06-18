use alef::backends::extendr::ExtendrBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
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
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
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

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
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
    }
}

fn make_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants,
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

fn make_variant(name: &str, fields: Vec<FieldDef>, is_tuple: bool) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type,
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
    }
}

#[test]
fn extendr_all_surface_types_get_conversions_comprehensive() {
    // Comprehensive scenario: a data enum with struct variants containing nested struct types,
    // returned from a function. All nested types must get From<core::T> impls to allow
    // conversion at every level.
    //
    // Simulates: enum VlmFallbackPolicy { Always, OnLowQuality { threshold: f64 }, ... }
    //     plus: enum Result { Ok(Metadata), Err { message: String, details: ErrorDetails } }

    let error_details = make_type(
        "ErrorDetails",
        vec![make_field("code", TypeRef::Primitive(PrimitiveType::U32))],
    );

    let metadata = make_type("Metadata", vec![make_field("format", TypeRef::String)]);

    // Enum with struct-variant enum that contains Named types
    let result_enum = make_enum(
        "Result",
        vec![
            make_variant(
                "Ok",
                vec![make_field("_0", TypeRef::Named("Metadata".to_string()))],
                true, // tuple variant
            ),
            make_variant(
                "Err",
                vec![
                    make_field("message", TypeRef::String),
                    make_field("details", TypeRef::Named("ErrorDetails".to_string())),
                ],
                false, // struct variant
            ),
        ],
    );

    // The containing struct uses Vec of the enum which contains nested types
    let report = make_type(
        "Report",
        vec![
            make_field("results", TypeRef::Vec(Box::new(TypeRef::Named("Result".to_string())))),
            make_field("metadata", TypeRef::Named("Metadata".to_string())),
        ],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![error_details, metadata, report],
        functions: vec![make_function("generate_report", TypeRef::Named("Report".to_string()))],
        enums: vec![result_enum],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    // All types should have From<core::T> impls:
    // 1. Report (direct return type)
    assert!(
        content.contains("impl From<test_lib::Report> for Report"),
        "Report should have From<core::Report> impl"
    );
    // 2. Metadata (used in Result enum variant and Report field)
    assert!(
        content.contains("impl From<test_lib::Metadata> for Metadata"),
        "Metadata should have From<core::Metadata> impl (in enum variant and struct field)"
    );
    // 3. ErrorDetails (used in Result struct variant)
    assert!(
        content.contains("impl From<test_lib::ErrorDetails> for ErrorDetails"),
        "ErrorDetails should have From<core::ErrorDetails> impl (in enum struct variant)"
    );
    // 4. Result enum should have conversion (lossy, data discarded)
    assert!(
        content.contains("impl From<test_lib::Result> for Result")
            || content.contains("impl From<Result> for test_lib::Result"),
        "Result enum should have conversion impl"
    );
}
