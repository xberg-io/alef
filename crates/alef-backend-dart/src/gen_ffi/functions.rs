use alef_core::ir::{FunctionDef, ParamDef, TypeRef};
use heck::ToLowerCamelCase;

use super::type_map::{
    call_arg_name, dart_callable_return, dart_callable_type, dart_public_return, dart_wrapper_param, native_param_type,
    native_return_type, unwrap_return_expr,
};

/// Emit a Dart function that resolves its C symbol via `_lib.lookupFunction`.
pub(super) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    free_symbol: &str,
    error_code_symbol: &str,
    out: &mut String,
) {
    use crate::template_env;
    if f.is_async {
        // TODO: dart:ffi async requires Isolate plumbing; deferred for Phase 3b.
        out.push_str(&template_env::render(
            "ffi_async_todo.jinja",
            minijinja::context! {
                name => f.name.as_str(),
            },
        ));
        return;
    }

    if !f.doc.is_empty() {
        let doc_lines: Vec<String> = f.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&template_env::render(
            "ffi_error_throws_doc.jinja",
            minijinja::context! {
                error_ty => error_ty.as_str(),
            },
        ));
    }

    let c_symbol = format!("{prefix}_{}", f.name);
    let fn_name = f.name.to_lower_camel_case();

    // Emit the native and Dart typedef pair.
    let native_params: Vec<String> = f.params.iter().map(native_param_type).collect();
    let native_return = native_return_type(&f.return_type);
    let dart_params: Vec<String> = f.params.iter().map(dart_callable_type).collect();
    let dart_return = dart_callable_return(&f.return_type);

    let typedef_native = format!("_{fn_name}Native");
    let typedef_dart = format!("_{fn_name}Dart");

    out.push_str(&template_env::render(
        "ffi_typedef_native_sig.jinja",
        minijinja::context! {
            typedef_native => typedef_native.as_str(),
            native_return => native_return.as_str(),
            native_params => native_params.join(", "),
        },
    ));
    out.push_str(&template_env::render(
        "ffi_typedef_dart_sig.jinja",
        minijinja::context! {
            typedef_dart => typedef_dart.as_str(),
            dart_return => dart_return.as_str(),
            dart_params => dart_params.join(", "),
        },
    ));
    out.push_str(&template_env::render(
        "ffi_function_lookup_sig.jinja",
        minijinja::context! {
            dart_return => dart_return.as_str(),
            dart_params => dart_params.join(", "),
            fn_name => fn_name.as_str(),
            typedef_native => typedef_native.as_str(),
            typedef_dart => typedef_dart.as_str(),
            c_symbol => c_symbol.as_str(),
        },
    ));

    // Emit the public wrapper function.
    let dart_wrapper_params: Vec<String> = f.params.iter().map(dart_wrapper_param).collect();
    let wrapper_return = dart_public_return(&f.return_type);

    out.push_str(&template_env::render(
        "ffi_wrapper_fn_open.jinja",
        minijinja::context! {
            wrapper_return => wrapper_return.as_str(),
            fn_name => fn_name.as_str(),
            dart_wrapper_params => dart_wrapper_params.join(", "),
        },
    ));

    // Allocate native strings for each string parameter.
    for p in &f.params {
        emit_param_alloc(p, out);
    }

    // Build the C call argument list.
    let call_args: Vec<String> = f.params.iter().map(call_arg_name).collect();
    let call_args_str = call_args.join(", ");

    if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&template_env::render(
            "ffi_call_void.jinja",
            minijinja::context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        if f.error_type.is_some() {
            out.push_str("  _checkError();\n");
        }
        emit_param_free_all(&f.params, out);
    } else {
        out.push_str(&template_env::render(
            "ffi_call_result.jinja",
            minijinja::context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        if f.error_type.is_some() {
            out.push_str("  _checkError();\n");
        }
        emit_param_free_all(&f.params, out);
        let ret_expr = unwrap_return_expr("_result", &f.return_type, free_symbol, error_code_symbol);
        out.push_str(&template_env::render(
            "ffi_return_value.jinja",
            minijinja::context! {
                ret_expr => ret_expr,
            },
        ));
    }

    out.push_str("}\n");
}

/// Allocate a native UTF-8 string for a string/path parameter.
fn emit_param_alloc(p: &ParamDef, out: &mut String) {
    use crate::template_env;
    let name = p.name.to_lower_camel_case();
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str(&template_env::render(
                "ffi_param_alloc_string.jinja",
                minijinja::context! {
                    name => name.as_str(),
                },
            ));
        }
        _ => {}
    }
}

/// Free all previously allocated native strings.
fn emit_param_free_all(params: &[ParamDef], out: &mut String) {
    use crate::template_env;
    for p in params {
        let name = p.name.to_lower_camel_case();
        match &p.ty {
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&template_env::render(
                    "ffi_param_free_string.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                    },
                ));
            }
            _ => {}
        }
    }
}
