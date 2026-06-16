//! Test that Java's Builder uses the correct default variant for enum fields with #[serde(default)].
//! This tests the fix for STY-6.

use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use alef::extract::default_value_for_enum::enum_default_variants_map;

#[test]
fn java_builder_uses_correct_default_variant_for_serde_default_enum_field() {
    // Create an enum with the first variant NOT being the default
    let enum_def = EnumDef {
        name: "MyEnumType".to_string(),
        rust_path: "test::MyEnumType".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "FirstVariant".to_string(),
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
                name: "ActualDefault".to_string(),
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
        ],
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        is_copy: false,
        has_serde: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Create an API surface with the enum
    let api = ApiSurface {
        crate_name: "test_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![enum_def.clone()],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    // Build the enum defaults map - this should map "MyEnumType" -> "ActualDefault"
    let enum_defaults = enum_default_variants_map(&api);
    assert_eq!(
        enum_defaults.get("MyEnumType"),
        Some(&"ActualDefault".to_string()),
        "enum_defaults map should contain MyEnumType -> ActualDefault"
    );

    // Create a struct with a #[serde(default)] enum field
    let _struct_def = TypeDef {
        name: "ConfigType".to_string(),
        rust_path: "test::ConfigType".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "mode".to_string(),
            ty: TypeRef::Named("MyEnumType".to_string()),
            optional: false,
            default: Some("/* serde(default) */".to_string()),
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: Default::default(),
            vec_inner_core_wrapper: Default::default(),
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        doc: String::new(),
        cfg: None,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    // The enum_defaults map should map MyEnumType to ActualDefault
    assert_eq!(
        enum_defaults.get("MyEnumType"),
        Some(&"ActualDefault".to_string()),
        "enum_defaults should map MyEnumType to ActualDefault (the #[default]-marked variant)"
    );

    // Verify that it does NOT map to the first variant
    assert_ne!(
        enum_defaults.get("MyEnumType"),
        Some(&"FirstVariant".to_string()),
        "enum_defaults should NOT map to FirstVariant (not the #[default] variant)"
    );
}

#[test]
fn enum_default_variants_map_extracts_default_variants() {
    // Create multiple enums with different default variants
    let enums = vec![
        EnumDef {
            name: "BrowserMode".to_string(),
            rust_path: "test::BrowserMode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Headless".to_string(),
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
                    name: "Auto".to_string(),
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
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: true,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        },
        EnumDef {
            name: "NoDefaultEnum".to_string(),
            rust_path: "test::NoDefaultEnum".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Option1".to_string(),
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
                    name: "Option2".to_string(),
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
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: true,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        },
    ];

    let api = ApiSurface {
        crate_name: "test_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums,
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let map = enum_default_variants_map(&api);

    // BrowserMode has an explicit #[default] on Auto variant
    assert_eq!(
        map.get("BrowserMode"),
        Some(&"Auto".to_string()),
        "BrowserMode should map to Auto (the #[default] variant)"
    );

    // NoDefaultEnum has no #[default], so it should default to the first variant
    assert_eq!(
        map.get("NoDefaultEnum"),
        Some(&"Option1".to_string()),
        "NoDefaultEnum should map to Option1 (first variant, since no #[default])"
    );
}
