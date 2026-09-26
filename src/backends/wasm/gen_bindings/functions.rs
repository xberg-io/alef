//! WASM free-function and utility code generation.

mod async_wrappers;
mod imports_helpers;
mod input_dto;
mod orchestration;
mod params;
mod returns;

pub(super) use imports_helpers::{emit_rustdoc, gen_env_shims};
pub(super) use input_dto::{gen_input_dto_for_type_with_cfg, should_have_input_dto};
pub(super) use orchestration::gen_function_with_emitted_dtos;
pub(super) use params::{borrow_opaque_param, format_param_unused};
pub(super) use returns::{gen_wasm_unimplemented_body, wasm_wrap_return};

#[cfg(test)]
use input_dto::dto_field_conversion;
#[cfg(test)]
use input_dto::gen_input_dto_for_type;
#[cfg(test)]
use returns::{to_turbofish_from, type_has_default};

#[cfg(test)]
#[path = "functions/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "functions/named_ref_delegation_tests.rs"]
mod named_ref_delegation_tests;
