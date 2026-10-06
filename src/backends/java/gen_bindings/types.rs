mod builders;
mod enums;
mod opaque;
mod records;
mod serializers;
mod shared;

pub(crate) use enums::{emits_get_value, emits_sealed_interface, gen_enum_class};
pub(crate) use opaque::gen_opaque_handle_class;
pub(crate) use records::gen_record_type;
pub(crate) use serializers::{
    gen_byte_array_serializer, gen_duration_millis_deserializer, gen_duration_millis_serializer,
    gen_json_mapper_factory,
};

#[cfg(test)]
mod builder_tests;
#[cfg(test)]
mod tests;
