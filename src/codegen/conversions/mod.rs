mod binding_to_core;
mod config;
mod core_to_binding;
mod enums;
pub(crate) mod helpers;

// Re-export all public items so callers continue to use `conversions::foo`.
pub use binding_to_core::{
    apply_core_wrapper_to_core, field_conversion_to_core, field_conversion_to_core_cfg, gen_from_binding_to_core,
    gen_from_binding_to_core_cfg, gen_from_lifetime_type_constructor,
};
pub use config::ConversionConfig;
pub use core_to_binding::{
    field_conversion_from_core, field_conversion_from_core_cfg, gen_from_core_to_binding, gen_from_core_to_binding_cfg,
};
pub use enums::{
    gen_enum_from_binding_to_core, gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding,
    gen_enum_from_core_to_binding_cfg,
};
pub use helpers::{
    apply_crate_remaps, binding_to_core_match_arm, build_type_path_map, can_generate_conversion,
    can_generate_enum_conversion, can_generate_enum_conversion_from_core, convertible_types, core_enum_path,
    core_enum_path_remapped, core_to_binding_convertible_types, core_to_binding_match_arm, core_type_path,
    core_type_path_remapped, field_references_excluded_type, has_sanitized_fields, input_type_names, is_tuple_variant,
    resolve_named_path,
};

#[cfg(test)]
mod tests;
