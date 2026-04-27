use alef_core::ir::{FunctionDef, ParamDef, TypeRef};
use heck::ToLowerCamelCase;

use super::type_map::{
    call_arg_name, dart_callable_return, dart_callable_type, dart_public_return, dart_wrapper_param,
    native_param_type, native_return_type, unwrap_return_expr,
};

/// Emit a Dart function that resolves its C symbol via `_lib.lookupFunction`.
pub(super) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    free_symbol: &str,
    error_code_symbol: &str,
    out: &mut String,
) {
    if f.is_async {
        // TODO: dart:ffi async requires Isolate plumbing; deferred for Phase 3b.
        out.push_str(&format!(
            "// TODO: async function '{}' is not supported in dart:ffi mode; deferred.\n",
            f.name
        ));
        return;
    }

    if !f.doc.is_empty() {
        for line in f.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&format!("/// Throws [StateError] on failure (was: {error_ty}).\n"));
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

    out.push_str(&format!(
        "typedef {typedef_native} = {native_return} Function({});\n",
        native_params.join(", ")
    ));
    out.push_str(&format!(
        "typedef {typedef_dart} = {dart_return} Function({});\n",
        dart_params.join(", ")
    ));
    out.push_str(&format!(
        "final {dart_return} Function({}) _{fn_name}Fn =\n",
        dart_params.join(", ")
    ));
    out.push_str(&format!(
        "    _lib.lookupFunction<{typedef_native}, {typedef_dart}>('{c_symbol}');\n\n"
    ));

    // Emit the public wrapper function.
    let dart_wrapper_params: Vec<String> = f.params.iter().map(dart_wrapper_param).collect();
    let wrapper_return = dart_public_return(&f.return_type);

    out.push_str(&format!(
        "{wrapper_return} {fn_name}({}) {{\n",
        dart_wrapper_params.join(", ")
    ));

    // Allocate native strings for each string parameter.
    for p in &f.params {
        emit_param_alloc(p, out);
    }

    // Build the C call argument list.
    let call_args: Vec<String> = f.params.iter().map(call_arg_name).collect();
    let call_args_str = call_args.join(", ");

    if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&format!("  _{fn_name}Fn({call_args_str});\n"));
        if f.error_type.is_some() {
            out.push_str("  _checkError();\n");
        }
        emit_param_free_all(&f.params, out);
    } else {
        out.push_str(&format!("  final _result = _{fn_name}Fn({call_args_str});\n"));
        if f.error_type.is_some() {
            out.push_str("  _checkError();\n");
        }
        emit_param_free_all(&f.params, out);
        let ret_expr = unwrap_return_expr("_result", &f.return_type, free_symbol, error_code_symbol);
        out.push_str(&format!("  return {ret_expr};\n"));
    }

    out.push_str("}\n");
}

/// Allocate a native UTF-8 string for a string/path parameter.
fn emit_param_alloc(p: &ParamDef, out: &mut String) {
    let name = p.name.to_lower_camel_case();
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str(&format!("  final {name}Native = {name}.toNativeUtf8();\n"));
        }
        _ => {}
    }
}

/// Free all previously allocated native strings.
fn emit_param_free_all(params: &[ParamDef], out: &mut String) {
    for p in params {
        let name = p.name.to_lower_camel_case();
        match &p.ty {
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&format!("  calloc.free({name}Native);\n"));
            }
            _ => {}
        }
    }
}
