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
    EnumDef {
        name: "Color".to_string(),
        rust_path: "my_crate::Color".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
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
                serde_untagged: false,
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
                serde_untagged: false,
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
        serde_content: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
        has_default: false,
    }
}

fn excluded_variant() -> EnumVariant {
    // Simulates a cfg-gated or #[alef(skip)]-annotated variant absent from the binding.
    EnumVariant {
        serde_untagged: false,
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

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "From<BindingEnum>→core: catch-all must not be emitted even when core has excluded variants — \
         the binding match is always exhaustive over the binding type.\n{binding_to_core}"
    );

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
        binding_enums_have_data: true,
        binding_tuple_form_for_variants: true,
        ..Default::default()
    };

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "Magnus From<BindingEnum>→core with excluded variants must not emit catch-all \
         (unreachable pattern under -D warnings).\n{binding_to_core}"
    );
}

fn cfg_gated_variant(name: &str, cfg: &str) -> EnumVariant {
    EnumVariant {
        serde_untagged: false,
        name: name.into(),
        fields: vec![],
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: Some(cfg.to_string()),
        version: Default::default(),
    }
}

/// The real regression this task fixes: a HOST-owned cfg-gated variant kept in
/// `enum_def.variants` (not `excluded_variants`) must not trigger a catch-all in either
/// direction. Its match arm carries the identical `#[cfg(...)]` guard as the variant itself
/// (`emit_cfg_gated_arm`), so the variant and arm always compile in or out together and the
/// match stays exhaustive under any single build -- a catch-all here is never reachable and
/// trips `-D warnings`' `unreachable_patterns` the moment the gating feature is active, which it
/// is by default once the binding crate forwards its cfg features (alef #464).
#[test]
fn host_owned_cfg_gated_variant_no_catch_all_in_either_direction() {
    let mut enum_def = make_color_enum();
    enum_def
        .variants
        .push(cfg_gated_variant("Purple", "feature = \"extra-colors\""));

    let config = ConversionConfig::default();

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        binding_to_core.contains("Purple"),
        "the host-owned cfg-gated variant's arm must still be emitted:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "From<BindingEnum>→core: a host-owned cfg-gated variant must not trigger a catch-all \
         (unreachable pattern under -D warnings) -- the arm and the variant share the identical \
         cfg gate.\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        core_to_binding.contains("Purple"),
        "the host-owned cfg-gated variant's arm must still be emitted:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("_ => Default::default()"),
        "From<CoreEnum>→binding: a host-owned cfg-gated variant must not trigger a catch-all \
         (unreachable pattern under -D warnings).\n{core_to_binding}"
    );
}

/// Contrast: a FOREIGN-owned cfg-gated variant (merged in from a `[[crates.source_crates]]`
/// crate whose feature this binding crate cannot declare) has its arm dropped entirely and
/// unconditionally -- see `emit_cfg_gated_arm`. That really does leave the match
/// non-exhaustive, so the catch-all is required in both directions.
#[test]
fn foreign_owned_cfg_gated_variant_still_needs_catch_all_in_either_direction() {
    let mut enum_def = make_color_enum();
    enum_def.rust_path = "dep_crate::Color".to_string();
    enum_def
        .variants
        .push(cfg_gated_variant("Purple", "feature = \"extra-colors\""));

    let config = ConversionConfig::default();

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("Purple"),
        "a foreign-crate cfg-gated variant's arm must be dropped, not referenced:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("_ => Default::default()"),
        "From<BindingEnum>→core: dropping a foreign-crate cfg-gated variant's arm leaves the \
         match non-exhaustive, so the catch-all must still be emitted.\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        !core_to_binding.contains("Purple"),
        "a foreign-crate cfg-gated variant's arm must be dropped, not referenced:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("_ => Default::default()"),
        "From<CoreEnum>→binding: dropping a foreign-crate cfg-gated variant's arm leaves the \
         match non-exhaustive, so the catch-all must still be emitted.\n{core_to_binding}"
    );
}
