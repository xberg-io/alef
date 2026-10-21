mod csharp;
mod extendr;
mod go;
mod java;
mod magnus;
mod napi;
mod php;
mod pyo3;
mod rustler;
mod shared;

pub use csharp::gen_csharp_record;
pub use extendr::gen_extendr_kwargs_constructor;
pub use go::gen_go_functional_options;
pub use java::gen_java_builder;
pub use magnus::gen_magnus_kwargs_constructor;
pub use napi::gen_napi_defaults_constructor;
pub use php::gen_php_kwargs_constructor;
pub(crate) use php::gen_php_kwargs_constructor_with_rest;
pub use pyo3::gen_pyo3_kwargs_constructor;
pub use rustler::{gen_rustler_kwargs_constructor, gen_rustler_kwargs_constructor_with_exclude};
pub(crate) use shared::validate_rust_default_functions;
pub use shared::{default_value_for_field, default_value_for_field_in_type};

#[cfg(test)]
mod tests;
