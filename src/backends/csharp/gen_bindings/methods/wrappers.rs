use super::super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::super::functions::{is_bytes_result_func, is_bytes_result_method};
use super::super::{
    bytes_len_arg, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, needs_param_teardown, returns_ptr,
};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::doc_emission;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Skip methods that take opaque handle FFI pointers as first arg but operate on non-opaque types.
/// These are validation/property functions that shouldn't be exposed as static methods.
/// Examples: header_metadata_is_valid, conversion_options_default (Rust naming, snake_case
pub(super) fn sanitize_doc_for_csharp(doc: &str) -> String {
    doc.lines()
        .filter_map(|line| {
            if line.trim().starts_with("use ") && line.contains("::") {
                return None;
            }
            Some(line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate a C# wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer
/// (`const TSLanguage *`) as an `IntPtr` for capsule types instead of an opaque alef handle.
/// The wrapper converts parameters, calls the C function, and constructs the host
/// `Language` (e.g. `new TreeSitter.Language(intPtr)`) from the raw pointer.
pub(super) fn gen_capsule_function_wrapper(
    func: &FunctionDef,
    exception_name: &str,
    _prefix: &str,
    cfg: &HostCapsuleTypeConfig,
) -> String {
    let mut out = String::with_capacity(1024);

    let func_cs_name = to_csharp_name(&func.name);
    doc_emission::emit_csharp_doc(&mut out, &func.doc, "        ", exception_name);

    let host_type = match cfg.required_host_type("Language", "csharp") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("        // ALEF ERROR: {e}\n"));
            return out;
        }
    };

    out.push_str(&format!("        public static {host_type} {func_cs_name}("));

    let param_strs: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let param_type = csharp_type(&p.ty);
            let param_name = p.name.to_lower_camel_case();
            format!("{param_type} {param_name}")
        })
        .collect();
    out.push_str(&param_strs.join(", "));
    out.push_str(")\n        {\n");

    let cs_native_name = to_csharp_name(&func.name);
    let c_params: Vec<String> = func.params.iter().map(|p| p.name.to_lower_camel_case()).collect();

    out.push_str("            var nativeResult = NativeMethods.");
    out.push_str(&cs_native_name);
    out.push('(');
    out.push_str(&c_params.join(", "));
    out.push_str(");\n");

    out.push_str("            if (nativeResult == IntPtr.Zero)\n");
    out.push_str("            {\n");
    if matches!(func.return_type, TypeRef::Optional(_)) {
        out.push_str("                return null;\n");
    } else {
        out.push_str("                throw GetLastError();\n");
    }
    out.push_str("            }\n");

    if func.error_type.is_some() {
        out.push_str("            if (NativeMethods.LastErrorCode() != 0)\n");
        out.push_str("            {\n");
        out.push_str("                throw GetLastError();\n");
        out.push_str("            }\n");
    }

    let construct = match cfg.construct_required("nativeResult", "Language", "csharp") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("            // ALEF ERROR: {e}\n"));
            out.push_str("        }\n");
            return out;
        }
    };
    out.push_str(&format!("            return {construct};\n"));

    out.push_str("        }\n");

    out
}

/// Generate a static wrapper method for a streaming method on an opaque type.
/// Delegates to the instance method on the opaque handle class.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_wrapper_function(
    func: &FunctionDef,
    exception_name: &str,
    _prefix: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    _has_visitor_callbacks: bool,
    types: &[crate::core::ir::TypeDef],
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::with_capacity(1024);

    let visible_params: Vec<crate::core::ir::ParamDef> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    doc_emission::emit_csharp_doc(&mut out, &func.doc, "    ", exception_name);
    for param in &visible_params {
        if !func.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&render(
                "param_doc.jinja",
                minijinja::context! { param_name, optional_text },
            ));
        }
    }

    out.push_str("    public static ");

    if func.is_async {
        if func.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            let return_type = csharp_type(&func.return_type);
            out.push_str(
                render("async_task_return_type.jinja", minijinja::context! { return_type }).trim_end_matches('\n'),
            );
        }
    } else if func.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&func.return_type));
    }

    out.push(' ');
    let func_name = to_csharp_name(&func.name);
    if func.is_async && !func_name.ends_with("Async") {
        out.push_str(&func_name);
        out.push_str("Async");
    } else {
        out.push_str(&func_name);
    }
    out.push('(');

    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(
                render(
                    "param_decl_optional.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_required.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    if is_bytes_result_func(func) {
        let cs_native_name = to_csharp_name(&func.name);
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        let mut args_block = String::new();
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => "            ", arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => "            ", param_name, optional => param.optional },
                ));
            }
        }
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(
            &mut cleanup_block,
            &visible_params,
            "            ",
            true_opaque_types,
            enum_names,
        );
        out.push_str(&render(
            "bytes_result_call.jinja",
            minijinja::context! {
                native_method_name => &cs_native_name,
                args_block => &args_block,
                cleanup_block => &cleanup_block,
            },
        ));
        out.push_str("    }\n\n");
        return out;
    }

    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    let cs_native_name = to_csharp_name(&func.name);

    let needs_outer_try = needs_param_teardown(&visible_params, true_opaque_types, enum_names);

    if func.is_async {
        if needs_outer_try {
            out.push_str("        try\n        {\n");
        }

        if func.return_type == TypeRef::Unit {
            out.push_str("            await Task.Run(() =>\n            {\n");
        } else {
            out.push_str("            return await Task.Run(() =>\n            {\n");
        }

        if func.return_type != TypeRef::Unit {
            out.push_str("                var nativeResult = ");
        } else {
            out.push_str("                ");
        }

        out.push_str(
            render(
                "native_call_start.jinja",
                minijinja::context! { method_name => &cs_native_name },
            )
            .trim_end_matches('\n'),
        );

        if visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let mut arg_parts: Vec<String> = Vec::new();
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(bytes_len_arg("(UIntPtr)", &param_name, param.optional));
                }
            }
            for (i, arg) in arg_parts.iter().enumerate() {
                out.push_str(render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if i < arg_parts.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("                );\n");
        }

        if func.return_type != TypeRef::Unit && returns_ptr(&func.return_type) {
            if matches!(func.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "                if (nativeResult == IntPtr.Zero)\n                {\n                    return null;\n                }\n",
                );
            } else {
                out.push_str(
                    "                if (nativeResult == IntPtr.Zero)\n                {\n                    throw GetLastError();\n                }\n",
                );
            }
        } else if func.error_type.is_some() {
            out.push_str(
                "                if (NativeMethods.LastErrorCode() != 0)\n                {\n                    throw GetLastError();\n                }\n",
            );
        }

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            "                ",
            enum_names,
            true_opaque_types,
            handle_returned_types,
        );
        emit_return_statement_indented(&mut out, &func.return_type, "                ");
        out.push_str("            });\n");

        if needs_outer_try {
            out.push_str("        }\n        finally\n        {\n");
            emit_named_param_teardown_indented(
                &mut out,
                &visible_params,
                "            ",
                true_opaque_types,
                enum_names,
            );
            out.push_str("        }\n");
        }
    } else {
        if needs_outer_try {
            out.push_str("        try\n        {\n");
        }

        if func.return_type != TypeRef::Unit {
            out.push_str("            var nativeResult = ");
        } else {
            out.push_str("            ");
        }

        out.push_str(
            render(
                "native_call_start.jinja",
                minijinja::context! { method_name => &cs_native_name },
            )
            .trim_end_matches('\n'),
        );

        if visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let mut arg_parts: Vec<String> = Vec::new();
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(bytes_len_arg("(UIntPtr)", &param_name, param.optional));
                }
            }
            for (i, arg) in arg_parts.iter().enumerate() {
                out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if i < arg_parts.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        let body_indent = if needs_outer_try { "            " } else { "        " };

        if func.return_type != TypeRef::Unit && returns_ptr(&func.return_type) {
            if matches!(func.return_type, TypeRef::Optional(_)) {
                out.push_str(&render(
                    "null_result_return.jinja",
                    minijinja::context! { indent => body_indent },
                ));
            } else {
                out.push_str(&render(
                    "last_error_throw.jinja",
                    minijinja::context! { indent => body_indent },
                ));
            }
        } else if func.error_type.is_some() {
            out.push_str(&render(
                "last_error_throw.jinja",
                minijinja::context! { indent => body_indent },
            ));
        }

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            body_indent,
            enum_names,
            true_opaque_types,
            handle_returned_types,
        );

        if needs_outer_try {
            emit_return_statement_indented(&mut out, &func.return_type, body_indent);
            out.push_str("        }\n        finally\n        {\n");
            emit_named_param_teardown_indented(
                &mut out,
                &visible_params,
                "            ",
                true_opaque_types,
                enum_names,
            );
            out.push_str("        }\n");
        } else {
            emit_named_param_teardown(&mut out, &visible_params, true_opaque_types, enum_names);
            emit_return_statement(&mut out, &func.return_type);
        }
    }

    out.push_str("    }\n\n");

    out
}

/// Generate a wrapper function for a function with a bridge field binding (e.g., visitor on options).
///
/// This handles functions where a trait bridge is injected into a struct field rather than
/// as a function parameter. The pattern:
/// 1. Extract the bridge value from the wrapped type (e.g., visitor from IHtmlVisitor)
/// 2. Serialize the options struct to JSON (skipping the bridge field)
/// 3. Deserialize into a native options handle
/// 4. If bridge present, create a bridge, inject into options, call convert, free bridge
/// 5. Otherwise, just call convert directly
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_wrapper_method(
    method: &MethodDef,
    exception_name: &str,
    _prefix: &str,
    type_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    types: &[crate::core::ir::TypeDef],
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::with_capacity(1024);

    let visible_params: Vec<crate::core::ir::ParamDef> = method
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    let sanitized_doc = sanitize_doc_for_csharp(&method.doc);
    doc_emission::emit_csharp_doc(&mut out, &sanitized_doc, "    ", exception_name);
    for param in &visible_params {
        if !method.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&render(
                "param_doc.jinja",
                minijinja::context! { param_name, optional_text },
            ));
        }
    }

    out.push_str("    public static ");

    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            let return_type = csharp_type(&method.return_type);
            out.push_str(
                render("async_task_return_type.jinja", minijinja::context! { return_type }).trim_end_matches('\n'),
            );
        }
    } else if method.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&method.return_type));
    }

    let method_name = to_csharp_name(&method.name);
    let method_cs_name = if method.is_async && !method_name.ends_with("Async") {
        format!("{}{}Async", type_name, method_name)
    } else {
        format!("{}{}", type_name, method_name)
    };
    out.push(' ');
    out.push_str(&method_cs_name);
    out.push('(');

    let has_receiver = !method.is_static && method.receiver.is_some();
    if has_receiver {
        out.push_str("IntPtr handle");
        if !visible_params.is_empty() {
            out.push_str(", ");
        }
    }

    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(
                render(
                    "param_decl_optional.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_required.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    let cs_native_name = format!("{}{}", csharp_type_name(type_name), to_csharp_name(&method.name));

    if is_bytes_result_method(method) {
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        let mut args_block = String::new();
        if has_receiver {
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => "            ", arg => "handle" },
            ));
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => "            ", arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => "            ", param_name, optional => param.optional },
                ));
            }
        }
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(
            &mut cleanup_block,
            &visible_params,
            "            ",
            true_opaque_types,
            enum_names,
        );
        out.push_str(&render(
            "bytes_result_call.jinja",
            minijinja::context! {
                native_method_name => &cs_native_name,
                args_block => &args_block,
                cleanup_block => &cleanup_block,
            },
        ));
        out.push_str("    }\n\n");
        return out;
    }

    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("        await Task.Run(() =>\n        {\n");
        } else {
            out.push_str("        return await Task.Run(() =>\n        {\n");
        }

        if method.return_type != TypeRef::Unit {
            out.push_str("            var nativeResult = ");
        } else {
            out.push_str("            ");
        }

        out.push_str(
            render(
                "native_call_start.jinja",
                minijinja::context! { method_name => &cs_native_name },
            )
            .trim_end_matches('\n'),
        );

        if !has_receiver && visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let mut arg_parts: Vec<String> = Vec::new();
            if has_receiver {
                arg_parts.push("handle".to_string());
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(bytes_len_arg("(UIntPtr)", &param_name, param.optional));
                }
            }
            for (i, arg) in arg_parts.iter().enumerate() {
                out.push_str(render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if i < arg_parts.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "            if (nativeResult == IntPtr.Zero)\n            {\n                return null;\n            }\n",
                );
            } else {
                out.push_str(
                    "            if (nativeResult == IntPtr.Zero)\n            {\n                throw GetLastError();\n            }\n",
                );
            }
        } else if method.error_type.is_some() {
            out.push_str(
                "            if (NativeMethods.LastErrorCode() != 0)\n            {\n                throw GetLastError();\n            }\n",
            );
        }

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "            ",
            enum_names,
            true_opaque_types,
            &HashSet::new(),
        );
        emit_named_param_teardown_indented(&mut out, &visible_params, "            ", true_opaque_types, enum_names);
        emit_return_statement_indented(&mut out, &method.return_type, "            ");
        out.push_str("        });\n");
    } else {
        if method.return_type != TypeRef::Unit {
            out.push_str("        var nativeResult = ");
        } else {
            out.push_str("        ");
        }

        out.push_str(
            render(
                "native_call_start.jinja",
                minijinja::context! { method_name => &cs_native_name },
            )
            .trim_end_matches('\n'),
        );

        if !has_receiver && visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let mut arg_parts: Vec<String> = Vec::new();
            if has_receiver {
                arg_parts.push("handle".to_string());
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(bytes_len_arg("(UIntPtr)", &param_name, param.optional));
                }
            }
            for (i, arg) in arg_parts.iter().enumerate() {
                out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if i < arg_parts.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("        );\n");
        }

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "        if (nativeResult == IntPtr.Zero)\n        {\n            return null;\n        }\n",
                );
            } else {
                out.push_str(
                    "        if (nativeResult == IntPtr.Zero)\n        {\n            throw GetLastError();\n        }\n",
                );
            }
        } else if method.error_type.is_some() {
            out.push_str(
                "        if (NativeMethods.LastErrorCode() != 0)\n        {\n            throw GetLastError();\n        }\n",
            );
        }

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "        ",
            enum_names,
            true_opaque_types,
            handle_returned_types,
        );
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types, enum_names);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::HostCapsuleTypeConfig;
    use crate::core::ir::{CoreWrapper, FunctionDef, ParamDef, TypeRef, VersionAnnotation};

    #[test]
    fn capsule_function_wrapper_uses_correct_pinvoke_name() {
        let func = FunctionDef {
            name: "get_language".to_string(),
            rust_path: "test::get_language".to_string(),
            original_rust_path: "test::get_language".to_string(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: CoreWrapper::default(),
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
            version: VersionAnnotation::default(),
        };

        let cfg = HostCapsuleTypeConfig {
            host_type: "TreeSitter.Language".to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: String::new(),
        };

        let code = gen_capsule_function_wrapper(&func, "TestException", "sample_ffi", &cfg);

        assert!(
            code.contains("NativeMethods.GetLanguage(name)"),
            "Generated code should call NativeMethods.GetLanguage, got:\n{}",
            code
        );
        assert!(
            !code.contains("NativeMethods.sample_ffi_get_language"),
            "Generated code should NOT call NativeMethods.sample_ffi_get_language (snake_case), got:\n{}",
            code
        );
    }
}
