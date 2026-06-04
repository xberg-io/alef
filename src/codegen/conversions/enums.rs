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

    // When the core enum has cfg-gated variants (excluded from the IR's `variants` list),
    // the Rust compiler sees those variants at compile time but the generated match arms
    // don't cover them, making the match non-exhaustive. Emit a wildcard arm in that case.
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();

    crate::codegen::template_env::render(
        "conversions/enum_from_binding_to_core",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => has_excluded_variants,
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

    // When the core enum has cfg-gated variants (excluded from the IR's `variants` list),
    // the Rust compiler sees those variants at compile time but the generated match arms
    // don't cover them, making the match non-exhaustive. Emit a wildcard arm in that case.
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();

    crate::codegen::template_env::render(
        "conversions/enum_from_core_to_binding",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => has_excluded_variants,
        },
    )
}
