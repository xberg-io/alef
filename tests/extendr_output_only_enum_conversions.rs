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
        version: Default::default(),
    }
}

fn make_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants,
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
fn extendr_output_only_enum_gets_both_conversions() {
    // Bug scenario: FormatMetadata is a flat data enum (has variants with single String fields)
    // that only appears in return types, never as a function parameter.
    //
    // Current code (line 651 in mod.rs):
    //   if input_types.contains(&e.name) && ... { emit binding→core }
    //
    // This would skip binding→core for output-only enums!
    // But flat data enums already emit From<core> at line 630, so we also need From<binding>
    // for round-trip safety.

    let format_enum = make_enum(
        "FormatMetadata",
        vec![
            make_variant("Html", vec![make_field("_0", TypeRef::String)], true),
            make_variant("Pdf", vec![make_field("_0", TypeRef::String)], true),
            make_variant("Unknown", vec![], false),
        ],
    );

    let container = make_type(
        "Result",
        vec![make_field("metadata", TypeRef::Named("FormatMetadata".to_string()))],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![container],
        functions: vec![make_function("process", TypeRef::Named("Result".to_string()))],
        enums: vec![format_enum],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
        ..Default::default()
    };

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    // Flat data enum should get both directions of conversion
    let has_core_to_binding = content.contains("impl From<test_lib::FormatMetadata> for FormatMetadata");
    let has_binding_to_core = content.contains("impl From<FormatMetadata> for test_lib::FormatMetadata");

    assert!(
        has_core_to_binding,
        "FormatMetadata should have From<core::FormatMetadata> impl (output-only enum)"
    );
    // This is the critical assertion — output-only enums should still get binding→core
    // for round-trip safety even if they never appear as function parameters.
    assert!(
        has_binding_to_core,
        "FormatMetadata should have From<binding::FormatMetadata> impl (even output-only)"
    );
}
