use crate::core::ir::EnumDef;

use super::ConversionConfig;
use super::helpers::{binding_to_core_match_arm_ext_cfg, core_enum_path_remapped, core_to_binding_match_arm_ext_cfg};

/// Generate `impl From<BindingEnum> for core::Enum` (binding -> core).
pub fn gen_enum_from_binding_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_binding_to_core_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingEnum> for core::Enum` with backend-specific config.
pub fn gen_enum_from_binding_to_core_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);

    // Pre-compute all arms for the template
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .map(|variant| {
            binding_to_core_match_arm_ext_cfg(
                &binding_name,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                enum_def.serde_untagged && config.binding_tuple_form_for_untagged_variants,
            )
        })
        .collect();

    // The match is on the *binding* enum, which only contains `enum_def.variants`
    // (excluded variants are absent from the binding type). Each variant in
    // `enum_def.variants` gets its own arm, so the match is always exhaustive over
    // the binding type. A wildcard `_ => Default::default()` arm is NEVER needed
    // here — emitting one when all variants are covered produces an "unreachable
    // pattern" error under `-D warnings`.
    //
    // Contrast with `gen_enum_from_core_to_binding_cfg` (core → binding), where
    // the match is on the *core* type and excluded variants require a catch-all.
    let needs_catch_all = false;

    crate::codegen::template_env::render(
        "conversions/enum_from_binding_to_core",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}

/// Generate `impl From<core::Enum> for BindingEnum` (core -> binding).
pub fn gen_enum_from_core_to_binding(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_core_to_binding_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<core::Enum> for BindingEnum` with backend-specific config.
pub fn gen_enum_from_core_to_binding_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);

    // Pre-compute all arms for the template
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .map(|variant| {
            core_to_binding_match_arm_ext_cfg(
                &core_path,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                enum_def.serde_untagged && config.binding_tuple_form_for_untagged_variants,
            )
        })
        .collect();

    // Emit a wildcard arm only when the core enum has cfg-gated variants that are absent
    // from the IR's `variants` list (stored in `excluded_variants` instead). In that case
    // the compiler sees those variants at compile time but the match has no arm for them,
    // so a `_ => Default::default()` catch-all keeps the match exhaustive.
    //
    // When all core variants ARE in `enum_def.variants`, every variant gets its own
    // explicit arm (unit variants → `Self::V`, tuple variants → `CoreT::V(..)`,
    // struct variants → `CoreT::V { .. }`). The match is exhaustive without a wildcard,
    // and emitting `_ => Default::default()` would produce an "unreachable pattern"
    // error under `-D warnings`.
    //
    // In particular, a unit-only binding with core struct variants (e.g. a NAPI
    // `string_enum` for a core enum that has data) does NOT need a catch-all: each
    // struct variant is matched by its own `CoreT::Variant { .. } => Self::Variant,` arm.
    let needs_catch_all = !enum_def.excluded_variants.is_empty();

    crate::codegen::template_env::render(
        "conversions/enum_from_core_to_binding",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}
