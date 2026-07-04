//! Emits the swift-bridge wrapper newtype structs and Rust crate shims.
//!
//! This facade preserves the historical `gen_rust_crate::wrappers` module path
//! while keeping wrapper generation split by concern.

mod constructors;
mod getters;
mod methods;
mod streaming_shims;

pub(crate) use constructors::{emit_type_constructor_shim, emit_type_wrapper};
pub(crate) use getters::is_unbridgeable_getter;
pub(crate) use methods::{emit_first_class_dto_method_wrappers, emit_type_method_shims};
pub(crate) use streaming_shims::emit_streaming_adapter_shims;
