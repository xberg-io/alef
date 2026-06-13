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

fn make_api(enums: Vec<EnumDef>, functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions,
        enums,
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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

fn make_enum_param_function(enum_name: &str) -> FunctionDef {
    FunctionDef {
        name: "select".to_string(),
        rust_path: "test_lib::select".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "mode".to_string(),
            ty: TypeRef::Named(enum_name.to_string()),
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
            core_wrapper: CoreWrapper::None,
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
    }
}

#[test]
fn extendr_unit_enum_conversions_use_local_templates() {
    let api = make_api(
        vec![make_enum(
            "Mode",
            vec![make_variant("Fast", vec![], false), make_variant("Slow", vec![], false)],
        )],
        vec![make_enum_param_function("Mode")],
    );

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    assert!(content.contains("impl From<Mode> for test_lib::Mode"), "{content}");
    assert!(content.contains("Mode::Fast => Self::Fast,"), "{content}");
    assert!(content.contains("Mode::Slow => Self::Slow,"), "{content}");
    assert!(content.contains("impl From<test_lib::Mode> for Mode"), "{content}");
    assert!(content.contains("test_lib::Mode::Fast => Self::Fast,"), "{content}");
    assert!(content.contains("test_lib::Mode::Slow => Self::Slow,"), "{content}");
    assert!(
        !content.contains("_ => Self::default(),"),
        "unit-only enum conversion must not emit a fallback arm:\n{content}"
    );
}

#[test]
fn extendr_ordinary_data_enum_conversions_preserve_lossy_behavior() {
    let api = make_api(
        vec![make_enum(
            "Event",
            vec![
                make_variant("Started", vec![], false),
                make_variant(
                    "Moved",
                    vec![
                        make_field("_0", TypeRef::Primitive(PrimitiveType::U32)),
                        make_field("_1", TypeRef::String),
                    ],
                    true,
                ),
                make_variant(
                    "Stopped",
                    vec![
                        make_field("code", TypeRef::Primitive(PrimitiveType::U32)),
                        make_field("reason", TypeRef::String),
                    ],
                    false,
                ),
            ],
        )],
        vec![make_enum_param_function("Event")],
    );

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    assert!(
        content.contains("Event::Moved => Self::Moved(Default::default(), Default::default()),"),
        "{content}"
    );
    assert!(
        content.contains("test_lib::Event::Moved(..) => Self::Moved,"),
        "{content}"
    );
    assert!(
        content.contains("Event::Stopped => Self::Stopped { code: Default::default(), reason: Default::default() },"),
        "{content}"
    );
    assert!(
        content.contains("test_lib::Event::Stopped { .. } => Self::Stopped,"),
        "{content}"
    );
    assert!(content.contains("_ => Self::default(),"), "{content}");
}

#[test]
fn extendr_enum_with_excluded_variants_emits_conversion_fallback() {
    let mut mode = make_enum(
        "Mode",
        vec![make_variant("Fast", vec![], false), make_variant("Slow", vec![], false)],
    );
    mode.excluded_variants.push(make_variant("Experimental", vec![], false));

    let api = make_api(vec![mode], vec![make_enum_param_function("Mode")]);

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    let fallback_count = content.matches("_ => Self::default(),").count();
    assert_eq!(
        fallback_count, 2,
        "excluded variants must add fallbacks to both enum conversion impls:\n{content}"
    );
}

#[test]
fn extendr_flat_data_enum_with_struct_variant_generates_from_core_impl() {
    // VlmFallbackPolicy: unit variants (Disabled, Always) + struct variant (OnLowQuality { quality_threshold: f64 })
    // is_flat_data_enum=true (has data, all data variants have 1 field)
    // can_flat_data_enum_round_trip=false (struct variant, not tuple)
    // has serde_tag="mode"
    // Previously: skipped all conversion generation (bug)
    // Fixed: still generates From<core> impl (struct variant data lost in binding, which is acceptable)
    let mut enum_def = make_enum(
        "FallbackPolicy",
        vec![
            make_variant("Disabled", vec![], false),
            make_variant(
                "OnLowQuality",
                vec![make_field("quality_threshold", TypeRef::Primitive(PrimitiveType::F64))],
                false, // struct variant, not tuple
            ),
            make_variant("Always", vec![], false),
        ],
    );
    enum_def.serde_tag = Some("mode".to_string());

    let api = make_api(vec![enum_def], vec![make_enum_param_function("FallbackPolicy")]);

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    // Should generate From<test_lib::FallbackPolicy> impl even though it's not round-trip safe
    assert!(
        content.contains("impl From<test_lib::FallbackPolicy> for FallbackPolicy"),
        "flat data enum with struct variant must generate From<core> impl:\n{content}"
    );
    // Unit variants should be converted directly with discriminator field
    assert!(
        content.contains("test_lib::FallbackPolicy::Disabled => Self { mode: \"Disabled\".to_string()"),
        "{content}"
    );
    assert!(
        content.contains("test_lib::FallbackPolicy::Always => Self { mode: \"Always\".to_string()"),
        "{content}"
    );
    // Struct variant data is lost, converted with .. pattern matching to discard fields
    assert!(
        content.contains("test_lib::FallbackPolicy::OnLowQuality { .. } => Self { mode: \"OnLowQuality\".to_string()"),
        "{content}"
    );
}
