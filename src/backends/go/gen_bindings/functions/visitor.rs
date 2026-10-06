use super::super::methods::gen_param_to_c;
use super::super::types::emit_type_doc;
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::{alef_handle_c_type, go_optional_type, go_type};
use crate::codegen::naming::{go_free_function_name, go_param_name, pascal_to_snake, to_go_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

/// Generate a Go wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer.
/// The wrapper converts parameters, calls the C function, and constructs the host
/// `Language` from the raw pointer — never an opaque alef handle.
///
/// `cfg.construct_expr` and `cfg.host_type` are required; this function returns an
/// error string beginning with `"// ALEF ERROR:"` when either is empty.
pub(in crate::backends::go::gen_bindings) fn gen_capsule_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
    cfg: &crate::core::config::HostCapsuleTypeConfig,
    reserved_type_names: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(1024);

    let func_go_name = go_free_function_name(&func.name, reserved_type_names);
    emit_type_doc(&mut out, &func_go_name, &func.doc, "calls the FFI function.");

    let mut param_strs: Vec<String> = Vec::new();
    for p in func.params.iter() {
        let param_type = if p.optional {
            go_optional_type(&p.ty).into_owned()
        } else {
            go_type(&p.ty).into_owned()
        };
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
    }
    let params_str = param_strs.join(", ");
    let host_type = match cfg.required_host_type("Language", "go") {
        Ok(t) => t.to_string(),
        Err(e) => return format!("// ALEF ERROR: {e}\n"),
    };
    let is_fallible = func.error_type.is_some();
    let ret_type_str = if is_fallible {
        format!(" ({host_type}, error)")
    } else {
        format!(" {host_type}")
    };

    out.push_str(&crate::backends::go::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => &ret_type_str,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "lock_os_thread.jinja",
        minijinja::Value::default(),
    ));

    let conv_fail_prefix = if is_fallible { "nil, " } else { "nil" };
    for param in func.params.iter() {
        out.push_str(&gen_param_to_c(
            param,
            conv_fail_prefix,
            false,
            ffi_prefix,
            opaque_names,
            enum_names,
            ffi_param_enum_names,
        ));
    }

    let ffi_name = format!("C.{}", c_symbols::free_function_symbol(ffi_prefix, &func.name));
    let c_params: Vec<String> = func
        .params
        .iter()
        .map(|p| go_param_name(&format!("c_{}", p.name)))
        .collect();
    let c_call = format!("{}({})", ffi_name, c_params.join(", "));

    let construct = match cfg.construct_required("cLang", "Language", "go") {
        Ok(c) => c,
        Err(e) => return format!("// ALEF ERROR: {e}\n"),
    };
    out.push_str(&format!("\tcLang := {c_call}\n"));
    if is_fallible {
        out.push_str("\tif err := lastError(); err != nil {\n\t\treturn nil, err\n\t}\n");
        out.push_str(&format!("\treturn {construct}, nil\n"));
    } else {
        out.push_str("\tif cLang == nil {\n\t\treturn nil\n\t}\n");
        out.push_str(&format!("\treturn {construct}\n"));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a custom wrapper for an options-field visitor bridge function.
///
/// When the configured options field is not nil, the wrapper delegates to the visitor helper.
/// Otherwise, it calls the base FFI function without attaching a visitor bridge.
pub(in crate::backends::go::gen_bindings) fn gen_convert_with_visitor_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    _value_only_types: &std::collections::HashSet<String>,
    bridge_cfg: &TraitBridgeConfig,
    reserved_type_names: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = go_free_function_name(&func.name, reserved_type_names);
    emit_type_doc(&mut out, &func_go_name, &func.doc, "runs the generated conversion.");

    let options_type = bridge_cfg
        .options_type
        .as_deref()
        .expect("go options-field bridge requires options_type");
    let options_field = bridge_cfg
        .resolved_options_field()
        .expect("go options-field bridge requires options_field or param_name");
    let options_param = func
        .params
        .iter()
        .find(|p| type_ref_named_type(&p.ty) == Some(options_type));
    let options_go_name = options_param.map(|p| go_param_name(&p.name));
    let options_field_go = to_go_name(options_field);
    let helper_name = format!("{}WithVisitorHelper", func.name.to_snake_case());
    let return_type_name = named_return_type(&func.return_type)
        .expect("go options-field visitor wrapper currently requires a named return type");
    let return_go_type = go_optional_type(&func.return_type).into_owned();
    let return_type_str = format!(" ({return_go_type}, error)");
    // NOTE: all `TypeRef::Named` values cross the FFI boundary as alef's scalar `AlefHandle`
    // (see `backends::ffi::type_map::c_return_optional`), never as an opaque pointer to the
    // options struct — so the local variable declared from `options_json_to_c.jinja` must use
    // the handle's C type name, not `{PREFIX}{options_type}`.
    let options_c_type = alef_handle_c_type(ffi_prefix);
    let options_type_snake = pascal_to_snake(options_type);
    let return_type_snake = pascal_to_snake(return_type_name);

    let mut param_strs: Vec<String> = Vec::new();
    for p in &func.params {
        let param_type = if p.optional {
            go_optional_type(&p.ty).into_owned()
        } else if let TypeRef::Named(name) = &p.ty {
            if opaque_names.contains(name.as_str()) {
                format!("*{}", go_type(&p.ty))
            } else {
                go_type(&p.ty).into_owned()
            }
        } else {
            go_type(&p.ty).into_owned()
        };
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
    }
    let params_str = param_strs.join(", ");

    out.push_str(&crate::backends::go::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => &return_type_str,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "lock_os_thread.jinja",
        minijinja::Value::default(),
    ));

    if let Some(options_var) = options_go_name.as_deref() {
        let helper_args = func
            .params
            .iter()
            .map(|p| go_param_name(&p.name))
            .chain(std::iter::once(format!("{options_var}.{options_field_go}")))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&crate::backends::go::template_env::render(
            "visitor_helper_guard.jinja",
            minijinja::context! {
                options_var => options_var,
                options_field_go => &options_field_go,
                helper_name => &helper_name,
                helper_args => &helper_args,
            },
        ));
    }

    let ffi_name = format!("C.{}", c_symbols::free_function_symbol(ffi_prefix, &func.name));

    let mut c_args = Vec::new();
    for param in &func.params {
        if type_ref_named_type(&param.ty) == Some(options_type) {
            c_args.push("cOptions".to_string());
            continue;
        }
        let go_name = go_param_name(&param.name);
        let c_name = go_param_name(&format!("c_{}", param.name));
        if matches!(param.ty, TypeRef::String | TypeRef::Path) {
            out.push_str(&crate::backends::go::template_env::render(
                "c_string_arg_setup.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_name => &go_name,
                },
            ));
            c_args.push(c_name);
        } else {
            c_args.push(go_name);
        }
    }

    if options_param.is_some() {
        let options_var = options_go_name.as_deref().expect("checked above");
        let from_json_fn = format!("{ffi_prefix}_{options_type_snake}_from_json");
        let free_fn = format!("{ffi_prefix}_{options_type_snake}_free");
        let options_description = options_type_snake.replace('_', " ");
        out.push_str(&crate::backends::go::template_env::render(
            "options_json_to_c.jinja",
            minijinja::context! {
                options_c_type => &options_c_type,
                options_var => options_var,
                from_json_fn => &from_json_fn,
                free_fn => &free_fn,
                options_description => &options_description,
            },
        ));

        out.push_str(&crate::backends::go::template_env::render(
            "ffi_ptr_call.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                c_args => c_args.join(", "),
            },
        ));
    } else {
        out.push_str(&crate::backends::go::template_env::render(
            "ffi_ptr_call.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                c_args => c_args.join(", "),
            },
        ));
    }

    // NOTE: `ptr` here is the return of `{{ ffi_name }}` for a named return type, which is
    // alef's scalar `AlefHandle`, not a pointer — compare it to the handle's zero value.
    out.push_str("\tif ptr == 0 {\n");
    out.push_str("\t\tif err := lastError(); err != nil {\n");
    out.push_str("\t\t\treturn nil, err\n");
    out.push_str("\t\t}\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion returned nil\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::backends::go::template_env::render(
        "free_type.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
            type_snake => &return_type_snake,
            ptr => "ptr",
        },
    ));
    out.push('\n');

    let to_json_fn = format!("{ffi_prefix}_{return_type_snake}_to_json");
    out.push_str(&crate::backends::go::template_env::render(
        "result_json_unmarshal.jinja",
        minijinja::context! {
            to_json_fn => &to_json_fn,
            use_prefix_free_string => true,
            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
            return_type_name => return_type_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));

    out
}

fn type_ref_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => type_ref_named_type(inner),
        _ => None,
    }
}

fn named_return_type(ty: &TypeRef) -> Option<&str> {
    type_ref_named_type(ty)
}
