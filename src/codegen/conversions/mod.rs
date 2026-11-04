mod binding_to_core;
mod config;
pub(crate) mod construction;
mod core_to_binding;
mod enums;
pub(crate) mod helpers;

// Re-export all public items so callers continue to use `conversions::foo`.
pub(crate) use binding_to_core::select_constructor;
pub use binding_to_core::{
    apply_core_wrapper_to_core, field_conversion_to_core, field_conversion_to_core_cfg, gen_from_binding_to_core,
    gen_from_binding_to_core_cfg, gen_from_lifetime_type_constructor,
};
pub use config::{ConversionConfig, WasmCamelRecasedEnum};
pub use core_to_binding::{
    field_conversion_from_core, field_conversion_from_core_cfg, gen_from_core_to_binding, gen_from_core_to_binding_cfg,
};
pub use enums::{
    VariantDeclaration, enum_conversion_needs_catch_all, enum_conversion_needs_catch_all_for_features,
    enum_variant_declaration, enum_variant_declaration_without_cfg_attribute, gen_enum_from_binding_to_core,
    gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding, gen_enum_from_core_to_binding_cfg,
};
pub use helpers::{
    apply_crate_remaps, build_type_path_map, can_generate_conversion, can_generate_enum_conversion,
    can_generate_enum_conversion_from_core, convertible_types, core_enum_path, core_enum_path_remapped,
    core_to_binding_convertible_types, core_to_binding_from_impl_emitted, core_type_path, core_type_path_remapped,
    field_references_excluded_type, has_sanitized_fields, input_type_names, is_tuple_variant, pyo3_from_json_eligible,
    resolve_named_path,
};

#[cfg(test)]
mod enum_cfg_gate_tests;
#[cfg(test)]
mod enum_sanitized_field_tests;
#[cfg(test)]
mod tests;
