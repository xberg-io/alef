mod clippy_allow;
mod eligibility;
mod enum_arms;
mod field_fragments;
mod newtypes;
mod paths;
mod primitives;
mod type_discovery;

pub(crate) use clippy_allow::{clippy_allow_attr_line, needs_clippy_allow};
pub(crate) use eligibility::is_tuple_type_name;
pub use eligibility::{
    can_generate_conversion, can_generate_enum_conversion, can_generate_enum_conversion_from_core, convertible_types,
    core_to_binding_convertible_types, core_to_binding_from_impl_emitted, has_sanitized_fields, is_newtype,
    is_tuple_variant, pyo3_from_json_eligible, variant_emits_tuple_form,
};
pub use enum_arms::{binding_to_core_match_arm_ext_cfg, core_to_binding_match_arm_ext_cfg};
pub(crate) use field_fragments::{
    sanitized_field_to_binding_expr, sanitized_map_field_to_core_expr, sanitized_vec_field_to_core_expr,
};
pub(crate) use newtypes::{
    apply_explicit_field_newtype_from_core, apply_explicit_field_newtype_to_core, apply_field_newtype_from_core,
    apply_field_newtype_to_core, apply_newtype_from_core, apply_newtype_to_core,
};
pub use paths::{
    apply_crate_remaps, build_type_path_map, core_enum_path, core_enum_path_remapped, core_type_path,
    core_type_path_remapped, resolve_named_path,
};
pub(crate) use primitives::{binding_prim_str, core_prim_str, needs_f64_cast, needs_i32_cast, needs_i64_cast};
pub use type_discovery::{field_references_excluded_type, input_type_names};
