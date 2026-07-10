use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::Language;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

use super::type_map::{
    call_arg_name, dart_callable_return, dart_callable_type, dart_param_name, dart_public_return, dart_wrapper_param,
    native_param_type, native_return_type, unwrap_return_expr,
};

/// Emit a Dart function that resolves its C symbol via `_lib.lookupFunction`.
pub(super) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    free_symbol: &str,
    error_code_symbol: &str,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    out: &mut String,
) {
    use crate::backends::dart::template_env;

    if let Some(cap) = capsule_return_config(f, capsule_types) {
        emit_capsule_function(f, prefix, cap, out);
        return;
    }

    if f.is_async {
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
    let fn_name = public_host_identifier(Language::Dart, PublicIdentifierKind::Function, &f.name);

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

    for p in &f.params {
        emit_param_alloc(p, out);
    }

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
    use crate::backends::dart::template_env;
    let name = dart_param_name(&p.name);
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
    use crate::backends::dart::template_env;
    for p in params {
        let name = dart_param_name(&p.name);
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

/// Returns the host capsule config when `func` returns a configured capsule type
/// (bare `Named` return type).
fn capsule_return_config<'a>(
    func: &FunctionDef,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> Option<&'a crate::core::config::HostCapsuleTypeConfig> {
    if let TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Emit a Dart wrapper function for a capsule type that returns the host-native grammar pointer.
///
/// The exported C symbol returns the host raw `const TSLanguage *`. The wrapper converts
/// parameters, calls the C function, and returns the raw `Pointer<Void>` (or the configured
/// host_type when set) from the pointer without opaque-handle wrapping.
fn emit_capsule_function(
    f: &FunctionDef,
    prefix: &str,
    cap: &crate::core::config::HostCapsuleTypeConfig,
    out: &mut String,
) {
    use crate::backends::dart::template_env;

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

    let c_symbol = format!("{prefix}_{}", f.name);
    let fn_name = public_host_identifier(Language::Dart, PublicIdentifierKind::Function, &f.name);

    let native_params: Vec<String> = f.params.iter().map(native_param_type).collect();
    let native_return = "Pointer<Void>".to_string();
    let dart_params: Vec<String> = f.params.iter().map(dart_callable_type).collect();
    let dart_return = "Pointer<Void>".to_string();

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

    let dart_wrapper_params: Vec<String> = f.params.iter().map(dart_wrapper_param).collect();

    let wrapper_return = if cap.host_type.is_empty() {
        "Pointer<Void>?".to_string()
    } else {
        format!("{}?", cap.host_type)
    };

    out.push_str(&template_env::render(
        "ffi_wrapper_fn_open.jinja",
        minijinja::context! {
            wrapper_return => wrapper_return.as_str(),
            fn_name => fn_name.as_str(),
            dart_wrapper_params => dart_wrapper_params.join(", "),
        },
    ));

    for p in &f.params {
        emit_param_alloc(p, out);
    }

    let call_args: Vec<String> = f.params.iter().map(call_arg_name).collect();
    let call_args_str = call_args.join(", ");

    out.push_str(&template_env::render(
        "ffi_call_result.jinja",
        minijinja::context! {
            fn_name => fn_name.as_str(),
            call_args_str => call_args_str.as_str(),
        },
    ));

    emit_param_free_all(&f.params, out);

    let default_construct = "Pointer<Void>.fromAddress(_result.address)";
    let construct = cap.construct("_result", default_construct);
    out.push_str("  if (_result == null || _result.address == 0) return null;\n");
    out.push_str(&format!("  return {construct};\n"));

    out.push_str("}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::HostCapsuleTypeConfig;
    use std::collections::HashMap;

    fn get_language_fn() -> FunctionDef {
        FunctionDef {
            name: "get_language".to_string(),
            rust_path: "sample_capsule::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn emit_capsule_function_returns_host_language_pointer() {
        let f = get_language_fn();
        let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            HostCapsuleTypeConfig {
                host_type: "Pointer<Void>".to_string(),
                package: String::new(),
                package_version: String::new(),
                construct_expr: String::new(),
            },
        );
        let mut out = String::new();
        emit_function(&f, "tsp", "", "", &capsule_types, &mut out);
        assert!(
            out.contains("Pointer<Void>?"),
            "capsule fn must return Pointer<Void>? type. Got:\n{out}"
        );
        assert!(
            out.contains("tsp_get_language"),
            "capsule fn must reference the C symbol. Got:\n{out}"
        );
        assert!(
            out.contains("if (_result == null || _result.address == 0) return null;"),
            "capsule fn must guard null pointer. Got:\n{out}"
        );
    }

    #[test]
    fn emit_capsule_function_uses_default_pointer_construct() {
        let f = get_language_fn();
        let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            HostCapsuleTypeConfig {
                host_type: String::new(),
                package: String::new(),
                package_version: String::new(),
                construct_expr: String::new(),
            },
        );
        let mut out = String::new();
        emit_function(&f, "tsp", "", "", &capsule_types, &mut out);
        assert!(
            out.contains("Pointer<Void>?"),
            "empty host_type should default to Pointer<Void>?"
        );
        assert!(
            out.contains("Pointer<Void>.fromAddress(_result.address)"),
            "should use default pointer construct when construct_expr is empty"
        );
    }
}
