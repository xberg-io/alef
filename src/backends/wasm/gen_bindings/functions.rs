//! WASM free-function and utility code generation.

mod async_wrappers;
mod imports_helpers;
mod input_dto;
mod orchestration;
mod params;
mod returns;

pub(super) use imports_helpers::{emit_rustdoc, gen_env_shims};
pub(super) use input_dto::{InputDtoConfig, gen_input_dto_for_type_with_cfg_at_path, should_have_input_dto};
pub(super) use orchestration::{gen_function_with_emitted_dtos_and_remaps, uses_input_dtos};
pub(super) use params::{
    borrow_opaque_param, format_param_unused, typeref_to_core_type_str, wasm_call_args, wasm_mapped_param_type,
    wasm_newtype_param_bindings, wasm_type_path_map,
};
pub(super) use returns::{
    gen_wasm_unimplemented_body, wasm_mapped_return_type, wasm_wrap_return, wrap_jsvalue_mapped_return,
};

#[cfg(test)]
use input_dto::dto_field_conversion;
#[cfg(test)]
use input_dto::gen_input_dto_for_type;
#[cfg(test)]
use input_dto::gen_input_dto_for_type_at_path;
#[cfg(test)]
use input_dto::gen_input_dto_for_type_with_cfg;
#[cfg(test)]
use input_dto::input_dto_field_conversion;
#[cfg(test)]
use orchestration::gen_function_with_emitted_dtos;
#[cfg(test)]
use returns::{to_turbofish_from, type_has_default};

#[cfg(test)]
mod async_wrapper_tests;
#[cfg(test)]
#[path = "functions/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "functions/named_ref_delegation_tests.rs"]
mod named_ref_delegation_tests;
