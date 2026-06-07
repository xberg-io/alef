//! `object {Crate}` namespace, bridge calls, and Kotlin type/enum/error code emission.
//!
//! This module is split by generation concern while preserving the original
//! `object_wrapper::*` API used by sibling Kotlin modules.

mod dto;
mod enums;
mod errors;
mod methods;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use dto::emit_type_with_imports;
pub(crate) use enums::emit_enum;
pub(crate) use errors::emit_error_type_with_imports;
pub(crate) use methods::{emit_function, format_param_with_imports};
pub(crate) use types::kotlin_type_with_string_imports;
