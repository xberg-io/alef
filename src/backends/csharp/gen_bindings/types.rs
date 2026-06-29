//! C# opaque handle and record type code generation.

mod bridge_fields;
mod constructors;
mod converters;
mod opaque;
mod records;

#[cfg(test)]
mod tests;

pub(crate) use converters::{gen_byte_array_to_int_array_converter, gen_json_leniency};
pub(super) use opaque::gen_opaque_handle;
pub(super) use records::gen_record_type;
