mod eligibility;
mod enum_arms;
mod field_fragments;
mod paths;
mod primitives;
mod type_discovery;

pub(crate) use eligibility::is_tuple_type_name;
pub use eligibility::{
    can_generate_conversion, can_generate_enum_conversion, can_generate_enum_conversion_from_core, convertible_types,
    core_to_binding_convertible_types, has_sanitized_fields, is_newtype, is_tuple_variant,
};
#[allow(unused_imports)]
pub use enum_arms::{
    binding_to_core_match_arm, binding_to_core_match_arm_ext, binding_to_core_match_arm_ext_cfg,
    core_to_binding_match_arm, core_to_binding_match_arm_ext, core_to_binding_match_arm_ext_cfg,
};
pub(crate) use field_fragments::sanitized_vec_field_to_core_expr;
pub use paths::{
    apply_crate_remaps, build_type_path_map, core_enum_path, core_enum_path_remapped, core_type_path,
    core_type_path_remapped, resolve_named_path,
};
pub(crate) use primitives::{binding_prim_str, core_prim_str, needs_f64_cast, needs_i32_cast, needs_i64_cast};
pub use type_discovery::{field_references_excluded_type, input_type_names};
