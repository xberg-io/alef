mod context;
mod conversions;
mod json_values;
mod nif_service;
mod params_returns;

pub(super) use context::get_module_info;
#[allow(unused_imports)]
pub(super) use conversions::gen_elixir_enum_module;
#[cfg(test)]
pub(super) use conversions::gen_elixir_opaque_module;
pub(super) use conversions::{
    flat_data_enum_discriminator, gen_elixir_enum_module_with_known_types, gen_elixir_opaque_module_with_types,
    gen_elixir_struct_module, is_flat_data_enum,
};
pub(crate) use json_values::elixir_safe_param_name;
pub(super) use json_values::elixir_typespec;
pub(super) use nif_service::{collect_types_for_nif_derives, gen_native_ex};
pub(super) use params_returns::{elixir_return_typespec, gen_rustler_unimplemented_body, map_return_type};

#[cfg(test)]
use json_values::elixir_field_name_with_type;

#[cfg(test)]
mod elixir_oracle;
#[cfg(test)]
mod tests;
