mod cfg_dedup;
mod orchestration;
mod params;
mod result_presence;
mod return_handling;
mod signatures;
mod support;

pub(super) use params::{
    ParamConversionContext, gen_param_conversion_with_enums, optional_borrowed_param_call_arg,
    param_has_explicit_newtype,
};

pub(super) use cfg_dedup::dedup_same_name_functions;
pub(super) use orchestration::{
    gen_cancellable_free_function, gen_cancellable_method_wrapper, gen_free_function, gen_method_wrapper,
    gen_streaming_method_wrapper,
};
pub(super) use result_presence::{gen_free_function_result_presence_wrapper, gen_method_result_presence_wrapper};
pub(super) use return_handling::{returns_bytes_out_params, returns_c_char};
pub(super) use signatures::{
    gen_free_function_len_companion, is_owned_default_constructor, should_skip_method_wrapper,
};

#[cfg(test)]
mod tests;
