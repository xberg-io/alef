use crate::backends::zig::gen_bindings::helpers::emit_cleaned_zig_doc;
use crate::codegen::c_consumer;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashSet;

use super::params::{
    emit_static_method_param_conversion, method_param_needs_from_json, param_zig_type_with_enums,
    static_method_c_arg_names,
};
use super::render;

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

    // `opaque_static_body.jinja` unconditionally returns `error.UnknownFfiError` on a null
    // handle, so the declared set must carry that member even when nothing else can fail —
    // otherwise the signature and the body disagree and the emitted Zig does not compile. ~keep
    let err_set = if body_needs_invalid_json {
        "error{OutOfMemory,InvalidJson,UnknownFfiError}"
    } else if body_needs_try {
        "error{OutOfMemory,UnknownFfiError}"
    } else {
        "error{UnknownFfiError}"
    };
    let return_ty = format!("{err_set}!{}", ty.name);

    out.push_str(&render(
        "opaque_static_signature.jinja",
        minijinja::context! {
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
        minijinja::context! {
            c_call => &c_call,
            upper_prefix => &upper_prefix,
            type_name => &ty.name,
        },
    ));
}
