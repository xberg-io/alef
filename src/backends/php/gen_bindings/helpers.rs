mod enum_defaults;
mod params;
mod primitives;
mod return_wrapping;
mod runtime;
mod struct_conversion;

#[allow(unused_imports)]
pub(crate) use enum_defaults::{gen_convertible_enum_tainted, gen_enum_tainted_from_binding_to_core};
pub(crate) use params::{
    gen_php_call_args, gen_php_call_args_with_let_bindings, gen_php_call_args_with_let_bindings_vec,
    gen_php_function_params, gen_php_named_let_bindings, has_enum_named_field, has_json_param, param_conversion_is_fallible,
    php_json_decode_expr, references_named_type, wrap_body_in_ok,
};
#[allow(unused_imports)]
pub(crate) use primitives::core_prim_str;
pub(crate) use return_wrapping::php_wrap_return;
pub(crate) use runtime::gen_tokio_runtime;
pub(crate) use struct_conversion::gen_php_lossy_binding_to_core_fields;
