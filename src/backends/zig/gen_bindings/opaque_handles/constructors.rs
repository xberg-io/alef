use crate::codegen::c_consumer;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::TypeDef;
use heck::AsSnakeCase;
use std::collections::HashSet;

use super::params::{ffi_ty_needs_dupez, ffi_ty_to_zig};
use super::render;

/// Emit a top-level `pub fn create_<type_snake>(allocator, params...) !TypeName`
/// constructor that wraps the `c.{prefix}_{type_snake}_new(...)` FFI symbol.
pub(crate) fn emit_opaque_constructor(
    ty: &TypeDef,
    prefix: &str,
    ctor: &ClientConstructorConfig,
    top_level_names: &HashSet<String>,
    out: &mut String,
) {
    let type_snake = AsSnakeCase(&ty.name).to_string();
    let upper_prefix = c_consumer::export_type_prefix(prefix);
    let has_string_param = ctor.params.iter().any(|p| ffi_ty_needs_dupez(&p.ty));

    out.push_str(&render(
        "opaque_constructor_doc.jinja",
        crate::alef_context! {
            type_name => &ty.name,
        },
    ));

    let alloc_param = if has_string_param {
        "allocator: std.mem.Allocator, "
    } else {
        ""
    };

    let renamed_params: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            if top_level_names.contains(&p.name) {
                format!("{}_arg", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect();

    let params_str = renamed_params
        .iter()
        .zip(ctor.params.iter())
        .map(|(renamed_name, p)| format!("{}: {}", renamed_name, ffi_ty_to_zig(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&render(
        "opaque_constructor_signature.jinja",
        crate::alef_context! {
            type_snake => &type_snake,
            alloc_param => alloc_param,
            params => &params_str,
            type_name => &ty.name,
        },
    ));

    for (renamed_name, p) in renamed_params.iter().zip(ctor.params.iter()) {
        if ffi_ty_needs_dupez(&p.ty) {
            let c_name = format!("{}_z", p.name);
            out.push_str(&render(
                "opaque_constructor_string_param.jinja",
                crate::alef_context! {
                    c_name => &c_name,
                    param_name => renamed_name,
                },
            ));
        }
    }

    let c_args = renamed_params
        .iter()
        .zip(ctor.params.iter())
        .map(|(renamed_name, p)| {
            if ffi_ty_needs_dupez(&p.ty) {
                format!("{}_z.ptr", p.name)
            } else {
                renamed_name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&render(
        "opaque_constructor_body.jinja",
        crate::alef_context! {
            prefix => prefix,
            type_snake => &type_snake,
            c_args => &c_args,
            upper_prefix => &upper_prefix,
            type_name => &ty.name,
        },
    ));
}
