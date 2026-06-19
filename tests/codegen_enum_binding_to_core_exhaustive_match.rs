//! Regression tests for the exhaustive-match catch-all bug in
//! `gen_enum_from_binding_to_core_cfg`.
//!
//! The `From<BindingEnum> for CoreEnum` conversion matches on the *binding*
//! enum, which only ever contains the non-excluded variants listed in
//! `enum_def.variants`. All variants are covered by explicit arms, making the
//! match unconditionally exhaustive. A wildcard `_ => Default::default()` arm
//! is therefore never reachable and must not be emitted — doing so produces
//! `error: unreachable pattern` under `-D warnings` (our CI policy).
//!
//! Contrast: `From<CoreEnum> for BindingEnum` matches on the *core* type,
//! which includes excluded variants not present in the binding. That direction
//! correctly keeps the catch-all when excluded variants exist.

use alef::codegen::conversions::{
    ConversionConfig, gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding_cfg,
};
use alef::core::ir::{EnumDef, EnumVariant};

fn make_color_enum() -> EnumDef {
    // Exhaustive unit enum — no excluded variants.
    EnumDef {
        name: "Color".to_string(),
        rust_path: "my_crate::Color".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Red".into(),
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
                name: "Green".into(),
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
                name: "Blue".into(),
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
        excluded_variants: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
        has_default: false,
    }
}

fn excluded_variant() -> EnumVariant {
    // Simulates a cfg-gated or #[alef(skip)]-annotated variant absent from the binding.
    EnumVariant {
        name: "Invisible".into(),
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
    }
}

/// Exhaustive unit enum — no excluded variants, no catch-all in either direction.
#[test]
fn exhaustive_unit_enum_no_catch_all_in_either_direction() {
    let enum_def = make_color_enum();
    let config = ConversionConfig::default();

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "From<BindingEnum>→core: catch-all must not appear for exhaustive unit enum.\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        !core_to_binding.contains("_ => Default::default()"),
        "From<CoreEnum>→binding: catch-all must not appear when there are no excluded variants.\n{core_to_binding}"
    );
}

/// Enum with excluded variants:
///   - From<CoreEnum>→binding MUST emit catch-all (core has more variants than binding).
///   - From<BindingEnum>→core must NOT emit catch-all (binding only has the non-excluded variants).
#[test]
fn excluded_variant_enum_binding_to_core_never_has_catch_all() {
    let mut enum_def = make_color_enum();
    enum_def.excluded_variants.push(excluded_variant());

    let config = ConversionConfig::default();

    // From<BindingEnum>→core: match on the binding type, which lacks "Invisible".
    // All binding arms are covered; catch-all is unreachable and must not be emitted.
    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "From<BindingEnum>→core: catch-all must not be emitted even when core has excluded variants — \
         the binding match is always exhaustive over the binding type.\n{binding_to_core}"
    );

    // From<CoreEnum>→binding: match on the core type, which includes "Invisible".
    // The catch-all covers the excluded variant; it IS required.
    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        core_to_binding.contains("_ => Default::default()"),
        "From<CoreEnum>→binding: catch-all must be emitted when core has excluded variants.\n{core_to_binding}"
    );
}

/// Magnus (`binding_enums_have_data = true`) with excluded variants — same rule applies.
/// This is the configuration that triggered the original bug: a data enum with a
/// cfg-gated variant marked `#[alef(skip)]` and compiled with that feature active,
/// making the `_ => Default::default()` arm unreachable under `-D warnings`.
#[test]
fn magnus_data_enum_with_excluded_variant_no_catch_all_in_binding_to_core() {
    let mut enum_def = make_color_enum();
    enum_def.excluded_variants.push(excluded_variant());

    let config = ConversionConfig {
        binding_enums_have_data: true,                  // Magnus
        binding_tuple_form_for_untagged_variants: true, // Magnus
        ..Default::default()
    };

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "Magnus From<BindingEnum>→core with excluded variants must not emit catch-all \
         (unreachable pattern under -D warnings).\n{binding_to_core}"
    );
}
