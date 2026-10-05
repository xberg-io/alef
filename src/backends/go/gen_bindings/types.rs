mod config_options;
mod enums;
mod field_shape;
mod helpers;
mod mapping;
mod structs;

pub(super) use config_options::gen_config_options;
pub(super) use enums::gen_enum_type;
pub(crate) use enums::is_passthrough_raw_message_enum;
pub(crate) use enums::{
    GoEnumRepresentation, GoStructEnumVariantField, go_adjacent_tagged_constructor,
    go_data_enum_untagged_variant_matches, go_data_enum_variant_scalar_tuple_field, go_data_enum_variant_struct,
    go_enum_constant_for_wire_value, go_enum_representation, go_struct_enum_tag_field, go_struct_enum_variant_fields,
};
pub(crate) use field_shape::{
    go_data_enum_variant_field, is_data_interface_struct_field_enum, is_unit_struct_field_enum,
};
#[cfg(test)]
pub(super) use helpers::is_tuple_field;
pub(crate) use helpers::needs_omitempty_pointer;
pub(super) use helpers::{
    emit_type_doc, gen_duration_millis_helper, gen_last_error_helper, gen_ptr_helper, gen_unmarshal_bytes_helper,
    primary_go_error,
};
pub(super) use mapping::{cgo_type_for_primitive, go_return_expr, primitive_max_sentinel};
pub(crate) use structs::go_struct_field_type;
pub(super) use structs::{gen_opaque_type, gen_opaque_type_free_only, gen_struct_type, go_struct_field_names};

#[cfg(test)]
#[path = "types/field_shape_tests.rs"]
mod field_shape_tests;

#[cfg(test)]
#[path = "types/externally_tagged_newtype_enum_tests.rs"]
mod externally_tagged_newtype_enum_tests;

#[cfg(test)]
#[path = "types/named_serde_default_tests.rs"]
mod named_serde_default_tests;

#[cfg(test)]
#[path = "types/sealed_variant_field_tests.rs"]
mod sealed_variant_field_tests;

#[cfg(test)]
#[path = "types/tests.rs"]
mod tests;
