//! WASM error type generation.
//!
//! WASM error conversion is handled by `crate::codegen::error_gen::gen_wasm_error_converter`.
//! This module is a thin re-export shim so the gen_bindings structure is consistent
//! across all backends.

/// Generate a WASM error converter for a single error type.
///
/// Delegates to `crate::codegen::error_gen::gen_wasm_error_converter`.
pub(super) fn gen_error_converter(error: &crate::core::ir::ErrorDef, core_import: &str) -> String {
    crate::codegen::error_gen::gen_wasm_error_converter(error, core_import)
}

/// Generate an opaque WASM struct + `#[wasm_bindgen] impl` block for the
/// whitelisted introspection methods on an error type.
///
/// Delegates to `crate::codegen::error_gen::gen_wasm_error_methods`.
/// Returns an empty string when `error.methods` is empty.
pub(super) fn gen_error_methods(error: &crate::core::ir::ErrorDef, core_import: &str, prefix: &str) -> String {
    crate::codegen::error_gen::gen_wasm_error_methods(error, core_import, prefix)
}
