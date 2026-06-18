//! Test that the default variant extraction works correctly for enums.
//! This tests the fix for BLK-6 and STY-6.

use alef::core::ir::{EnumDef, EnumVariant};
use alef::extract::default_value_for_enum::default_variant_name;

#[test]
fn php_enum_default_variant_uses_marked_variant() {
    // Create an enum with multiple variants where the second one is marked #[default]
    let enum_def = EnumDef {
        name: "TestEnum".to_string(),
        rust_path: "test::TestEnum".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "First".to_string(),
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
                name: "Second".to_string(),
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
        methods: vec![],
        doc: String::new(),
        cfg: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        is_copy: false,
        has_serde: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // The helper should return the #[default]-marked variant, not the first one
    let default_variant = default_variant_name(&enum_def);
    assert_eq!(
        default_variant,
        Some("Second".to_string()),
        "default_variant_name should return the #[default]-marked variant (Second), not first variant"
    );
}

#[test]
fn enum_default_variant_falls_back_to_first_when_no_default_marker() {
    // Create an enum with no #[default] marker
    let enum_def = EnumDef {
        name: "NoDefaultEnum".to_string(),
        rust_path: "test::NoDefaultEnum".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "First".to_string(),
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
                name: "Second".to_string(),
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

    // Without #[default], should fall back to the first variant
    let default_variant = default_variant_name(&enum_def);
    assert_eq!(
        default_variant,
        Some("First".to_string()),
        "default_variant_name should fall back to first variant when no #[default] is marked"
    );
}
