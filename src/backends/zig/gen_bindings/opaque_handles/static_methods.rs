use crate::backends::zig::gen_bindings::errors::{UNMATCHED_ERROR_SET, resolve_zig_error_type};
use crate::backends::zig::gen_bindings::helpers::emit_cleaned_zig_doc;
use crate::codegen::c_consumer;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashSet;

use super::params::{
    emit_static_method_param_conversion, method_param_needs_from_json, param_zig_type_with_enums,
    static_method_c_arg_names,
};
use super::{null_handle_return, render};

/// Emit a static method (constructor) on an opaque handle type.
///
/// The FFI backend emits static constructors like `{prefix}_route_builder_new(method: i32, path: *const c_char)`
/// where enum parameters are passed as i32 discriminants. This function emits the Zig wrapper as a top-level
/// function that marshals enum parameters to i32 using `@intFromEnum()` and calls the C FFI symbol.
pub(super) fn emit_opaque_static_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    declared_errors: &[String],
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &method.doc, "");

    let method_snake = AsSnakeCase(&method.name).to_string();
    let type_snake = AsSnakeCase(&ty.name).to_string();
    let upper_prefix = c_consumer::export_type_prefix(prefix);

    let params_str = method
        .params
        .iter()
        .map(|p| {
            let ty_str = param_zig_type_with_enums(&p.ty, p.optional, struct_names, enum_names);
            format!("{}: {}", p.name, ty_str)
        })
        .collect::<Vec<_>>()
        .join(", ");

    let body_needs_try = method.params.iter().any(|p| {
        matches!(
            &p.ty,
            TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_)
        )
    });
    let body_needs_invalid_json = method
        .params
        .iter()
        .any(|p| method_param_needs_from_json(p, struct_names));

    // Every null handle reads the native error, including cancellation; a declared Rust
    // error type additionally determines the available typed variants. ~keep
    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));
    let return_ty = match &zig_error_type {
        Some(err_ty) => {
            let mut members = Vec::new();
            if body_needs_try {
                members.push("OutOfMemory");
            }
            if body_needs_invalid_json {
                members.push("InvalidJson");
            }
            if members.is_empty() {
                format!("{err_ty}!{}", ty.name)
            } else {
                format!("({err_ty}||error{{{}}})!{}", members.join(","), ty.name)
            }
        }
        None => {
            let err_set = if body_needs_invalid_json {
                "error{OutOfMemory,InvalidJson,UnknownFfiError,Cancelled}"
            } else {
                UNMATCHED_ERROR_SET
            };
            format!("{err_set}!{}", ty.name)
        }
    };
    let null_return = null_handle_return(zig_error_type.as_deref());

    out.push_str(&render(
        "opaque_static_signature.jinja",
        crate::alef_context! {
            method_snake => &method_snake,
            type_snake => &type_snake,
            params => &params_str,
            return_ty => &return_ty,
        },
    ));

    for p in &method.params {
        emit_static_method_param_conversion(p, prefix, struct_names, enum_names, out);
    }

    let mut c_args = Vec::new();
    for p in &method.params {
        c_args.extend(static_method_c_arg_names(p, struct_names, enum_names));
    }
    let c_call = format!(
        "c.{prefix}_{type_snake}_{method_snake}({args})",
        args = c_args.join(", ")
    );

    out.push_str(&render(
        "opaque_static_body.jinja",
        crate::alef_context! {
            c_call => &c_call,
            upper_prefix => &upper_prefix,
            type_name => &ty.name,
            null_return => &null_return,
        },
    ));
}
