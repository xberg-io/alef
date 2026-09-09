//! C# opaque handle and record type code generation.

mod bridge_fields;
pub(crate) mod constructors;
mod converters;
mod opaque;
mod records;

#[cfg(test)]
mod named_serde_default_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_nested_struct_defaults;

pub(crate) use converters::{
    gen_byte_array_to_int_array_converter, gen_duration_millis_converter, gen_ffi_json_extensions, gen_json_leniency,
};
pub(super) use opaque::gen_opaque_handle;
pub(super) use records::gen_record_type;
