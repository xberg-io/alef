mod config_options;
mod enums;
mod helpers;
mod mapping;
mod structs;

pub(super) use config_options::gen_config_options;
pub(super) use enums::{gen_enum_type, is_passthrough_raw_message_enum};
pub(super) use helpers::{
    emit_type_doc, gen_last_error_helper, gen_ptr_helper, gen_unmarshal_bytes_helper, is_tuple_field,
};
pub(super) use mapping::{cgo_type_for_primitive, go_return_expr, primitive_max_sentinel};
pub(super) use structs::{gen_opaque_type, gen_opaque_type_free_only, gen_struct_type};

#[cfg(test)]
#[path = "types/tests.rs"]
mod tests;
