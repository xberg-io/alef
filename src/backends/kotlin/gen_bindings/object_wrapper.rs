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

// `tests.rs` above is already at this repo's 1,000-line file-size cap (grandfathered), so new
// coverage for the `&mut` DTO write-back fix (issue #380) lives in its own module instead of
// growing that file.
#[cfg(test)]
mod mut_writeback_tests;

// Same reason as `mut_writeback_tests` above: task #558's optional-enum-default security fix
// gets its own module instead of growing the already-over-cap `tests.rs`.
#[cfg(test)]
mod optional_enum_default_tests;

// Same reason again: the named-`#[serde(default = "path")]` deferral coverage gets its own module
// rather than growing the already-over-cap `tests.rs`.
#[cfg(test)]
mod named_serde_default_tests;

pub(crate) use dto::emit_type_with_imports;
pub(crate) use enums::emit_enum;
pub(crate) use errors::emit_error_type_with_imports;
pub(crate) use methods::{emit_function, format_param_with_imports};
pub(crate) use types::{default_constructible_type_names, kotlin_field_default, kotlin_type_with_string_imports};
