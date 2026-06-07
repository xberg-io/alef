//! Python API wrapper function generation: `api.py`.

mod async_wrappers;
mod converters;
mod function_wrappers;
mod helper_type_mapping;
mod orchestration;
mod return_error;
mod signature_params;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(super) use helper_type_mapping::classify_param_type;
pub(super) use orchestration::gen_api_py;
#[allow(unused_imports)]
pub(super) use signature_params::emit_param_conversion;
