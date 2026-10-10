use super::call_args::{is_bytes_param, needs_vec_f32_conversion};
use crate::core::ir::ParamDef;

pub(super) fn gen_vec_f32_conversion_bindings(params: &[ParamDef]) -> String {
    let mut bindings = String::new();
    for p in params {
        if needs_vec_f32_conversion(&p.ty) && p.is_ref {
            let conv_name = format!("{}_f32", p.name);
            bindings.push_str(&crate::backends::napi::template_env::render(
                "vec_f32_conversion_binding.jinja",
                crate::alef_context! {
                    conv_name => conv_name,
                    param_name => &p.name,
                },
            ));
        }
    }
    bindings
}

/// Generate let bindings for napi::Buffer parameters that need conversion to Vec<u8>.
/// NAPI gives us napi::Buffer which dereferences to &[u8], but we need Vec<u8>.
///
/// For optional params the binding type is `Option<Buffer>`, which has no `.to_vec()`; emit
/// `.map(|b| b.to_vec())` so the result is `Option<Vec<u8>>` and a later `.as_deref()` at the
/// call site yields `Option<&[u8]>`.
pub(super) fn gen_napi_buffer_conversion_bindings(params: &[ParamDef]) -> String {
    let mut bindings = String::new();
    for p in params {
        if is_bytes_param(&p.ty) {
            let template = if p.optional {
                "buffer_conversion_binding_optional.jinja"
            } else {
                "buffer_conversion_binding.jinja"
            };
            bindings.push_str(&crate::backends::napi::template_env::render(
                template,
                crate::alef_context! {
                    param_name => &p.name,
                },
            ));
        }
    }
    bindings
}
