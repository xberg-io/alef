mod host_langs;
mod napi_wasm;
mod native;
mod pyo3;
mod shared;

pub use host_langs::{
    gen_csharp_error_types, gen_go_error_struct, gen_go_error_types, gen_go_sentinel_errors, gen_java_error_types,
    go_error_sentinel_name,
};
pub use napi_wasm::{
    gen_napi_error_class, gen_napi_error_converter, gen_napi_error_types, gen_wasm_error_converter,
    gen_wasm_error_methods, napi_converter_fn_name, wasm_converter_fn_name,
};
pub use native::{
    gen_ffi_error_codes, gen_ffi_error_methods, gen_magnus_error_converter, gen_magnus_error_methods_struct,
    gen_php_error_converter, gen_php_error_methods_impl, gen_rustler_error_converter, magnus_converter_fn_name,
    magnus_error_methods_registrations, php_converter_fn_name, rustler_converter_fn_name,
};
pub use pyo3::{
    converter_fn_name, gen_pyo3_error_converter, gen_pyo3_error_methods_impl, gen_pyo3_error_registration,
    gen_pyo3_error_types, pyo3_error_has_methods, pyo3_error_info_field_specs, pyo3_error_info_fn_name,
    pyo3_error_info_struct_name,
};
pub use shared::{acronym_aware_snake_phrase, python_exception_name, strip_thiserror_placeholders};

#[cfg(test)]
#[path = "error_gen/tests.rs"]
mod tests;
