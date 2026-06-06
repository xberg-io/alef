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

    // Emit a wildcard arm when:
    // 1. The core enum has cfg-gated variants (excluded from the IR's `variants` list), OR
    // 2. The binding enum is unit-only but the core enum has struct-variants with data
    //    (e.g., JSON-passthrough wrapper struct binding matching a struct-variant core enum).
    //    The compiler sees all core variants at compile time, so we must cover unrepresented ones.
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();
    let binding_is_unit_only = !config.binding_enums_have_data;
    let core_has_struct_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty() && !v.is_tuple);
    let needs_catch_all = has_excluded_variants || (binding_is_unit_only && core_has_struct_variants);

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

    // Emit a wildcard arm when:
    // 1. The core enum has cfg-gated variants (excluded from the IR's `variants` list), OR
    // 2. The binding enum is unit-only but the core enum has struct-variants with data
    //    (e.g., JSON-passthrough wrapper struct binding matching a struct-variant core enum).
    //    The compiler sees all core variants at compile time, so we must cover unrepresented ones.
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();
    let binding_is_unit_only = !config.binding_enums_have_data;
    let core_has_struct_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty() && !v.is_tuple);
    let needs_catch_all = has_excluded_variants || (binding_is_unit_only && core_has_struct_variants);

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
