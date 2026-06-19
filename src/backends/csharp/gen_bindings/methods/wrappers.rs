use super::super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::super::functions::{is_bytes_result_func, is_bytes_result_method};
use super::super::{
    emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented, is_bridge_param,
    native_call_arg, needs_param_teardown, returns_ptr,
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
            // Preserve the line as-is — don't strip backticks or blank lines.
            // The emit_csharp_doc function will handle proper sanitization
            // of Rust idioms, intra-doc links, and XML escaping.
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
    prefix: &str,
    cfg: &HostCapsuleTypeConfig,
) -> String {
    let mut out = String::with_capacity(1024);

    let func_cs_name = to_csharp_name(&func.name);
    doc_emission::emit_csharp_doc(&mut out, &func.doc, "        ", exception_name);

    // Return type is the host capsule type (e.g., "TreeSitter.Language")
    let host_type = if cfg.host_type.is_empty() {
        "IntPtr".to_string()
    } else {
        cfg.host_type.clone()
    };

    out.push_str(&format!("        public static {host_type} {func_cs_name}("));

    // Parameters (capsule functions typically have only simple scalar/string params)
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

    // Call the native P/Invoke function with the C function name
    let c_func_name = format!("{}_{}", prefix, func.name.to_lowercase());
    let cs_native_name = csharp_type_name(&c_func_name);
    let c_params: Vec<String> = func.params.iter().map(|p| p.name.to_lower_camel_case()).collect();

    out.push_str("            var nativeResult = NativeMethods.");
    out.push_str(&cs_native_name);
    out.push('(');
    out.push_str(&c_params.join(", "));
    out.push_str(");\n");

    // Guard null (grammar not found — return null for Optional, throw for required)
    out.push_str("            if (nativeResult == IntPtr.Zero)\n");
    out.push_str("            {\n");
    if matches!(func.return_type, TypeRef::Optional(_)) {
        out.push_str("                return null;\n");
    } else {
        out.push_str("                throw GetLastError();\n");
    }
    out.push_str("            }\n");

    // For fallible capsule functions, check error code after successful pointer return.
    // This mirrors the error handling pattern used by non-capsule fallible functions,
    // ensuring that Rust Result<T, E> errors are properly surfaced as exceptions.
    if func.error_type.is_some() {
        out.push_str("            if (NativeMethods.LastErrorCode() != 0)\n");
        out.push_str("            {\n");
        out.push_str("                throw GetLastError();\n");
        out.push_str("            }\n");
    }

    // Construct the host Language from the raw pointer.
    // The `{ptr}` placeholder is replaced with the variable name holding the IntPtr.
    let default_construct = "new TreeSitter.Language({ptr})";
    let construct = cfg.construct("nativeResult", default_construct);
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

    // Collect visible params (non-bridge) for the public C# signature.
    let visible_params: Vec<crate::core::ir::ParamDef> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    // XML doc comment using shared doc emission
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

    // Return type — use async Task<T> for async methods
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

    // Parameters (bridge params stripped from public signature)
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

    // Null checks for required string/object parameters.
    // Enums are value types in C# — ThrowIfNull on them triggers CA2264.
    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    // Result<Vec<u8>> uses the out-param convention — emit specialized body and return early.
    if is_bytes_result_func(func) {
        let cs_native_name = to_csharp_name(&func.name);
        // Emit setup for Named and Bytes parameters before calling the native method
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        // Build the args block for the template: each arg on its own indented line with trailing comma.
        let mut args_block = String::new();
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => "            ", arg },
            ));
            // For byte-slice input parameters, emit the length argument immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => "            ", param_name },
                ));
            }
        }
        // Build cleanup block for try-finally
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

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    // Method body - delegation to native method with proper marshalling
    let cs_native_name = to_csharp_name(&func.name);

    let needs_outer_try = needs_param_teardown(&visible_params, true_opaque_types, enum_names);

    if func.is_async {
        // Async: wrap in Task.Run for non-blocking execution. CS1997 disallows
        // `return await Task.Run(...)` in an `async Task` (non-generic) method,
        // so for unit returns we drop the `return`.

        // If we allocate temporary handles, wrap the native call in try/finally
        // so cleanup also runs when the native call reports failure.
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
                // For byte-slice input parameters, emit the length argument immediately after.
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(format!("(UIntPtr){param_name}.Length"));
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

        // Check for FFI error (null result means the call failed).
        // For Optional(_) return types, null means None (not found), not an error.
        // For numeric Result-returning functions, native returns a sentinel value (0) on error
        // and `LastErrorCode()` is set — Rust FFI clears it at every call entry, so a non-zero
        // value here unambiguously indicates the just-completed call failed.
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

        // Close outer try-finally if needed
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
        // Sync: wrap in try-finally if we have cleanup to do
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
                // For byte-slice input parameters, emit the length argument immediately after.
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(format!("(UIntPtr){param_name}.Length"));
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

        // Check for FFI error (null result means the call failed).
        // Pointer returns use IntPtr.Zero as a sentinel; numeric Result returns surface failure
        // via `LastErrorCode()`, which the Rust FFI clears at every call entry so a non-zero
        // value here unambiguously indicates the just-completed call failed.
        // For Optional(_) return types, null means None (not found), not an error.
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

    // Collect visible params (non-bridge) for the public C# signature.
    let visible_params: Vec<crate::core::ir::ParamDef> = method
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    // XML doc comment using shared doc emission
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

    // The wrapper class is always `static class`, so all methods must be static.
    out.push_str("    public static ");

    // Return type — use async Task<T> for async methods
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

    // Prefix method name with type name to avoid collisions (e.g., MetadataConfigDefault)
    let method_name = to_csharp_name(&method.name);
    let method_cs_name = if method.is_async && !method_name.ends_with("Async") {
        format!("{}{}Async", type_name, method_name)
    } else {
        format!("{}{}", type_name, method_name)
    };
    out.push(' ');
    out.push_str(&method_cs_name);
    out.push('(');

    // Non-static methods need a `handle` parameter that the wrapper threads to
    // the native receiver. Without this, the public method has no way to refer
    // to the instance and calls NativeMethods.{Method}() one argument short.
    let has_receiver = !method.is_static && method.receiver.is_some();
    if has_receiver {
        out.push_str("IntPtr handle");
        if !visible_params.is_empty() {
            out.push_str(", ");
        }
    }

    // Parameters (bridge params stripped from public signature)
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

    // Null checks for required string/object parameters.
    // Enums are value types in C# — ThrowIfNull on them triggers CA2264.
    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    let cs_native_name = format!("{}{}", csharp_type_name(type_name), to_csharp_name(&method.name));

    // Result<Vec<u8>> uses the out-param convention — emit specialized body and return early.
    if is_bytes_result_method(method) {
        // Emit setup for Named and Bytes parameters before calling the native method
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        // Build the args block: receiver (if any) then visible params, each with trailing comma.
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
            // For byte-slice input parameters, emit the length argument immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => "            ", param_name },
                ));
            }
        }
        // Build cleanup block for try-finally
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

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    // Method body - delegation to native method with proper marshalling.
    // Use the type-prefixed name to match the P/Invoke declaration, which includes the type
    // name to avoid collisions between different types with identically-named methods
    // (e.g. BrowserConfig::default and CrawlConfig::default).

    if method.is_async {
        // Async: wrap in Task.Run. For unit returns drop the `return` so CS1997 (async Task
        // method can't `return await` of non-generic Task) does not fire.
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
            // Build all argument parts (including byte-length args)
            let mut arg_parts: Vec<String> = Vec::new();
            if has_receiver {
                arg_parts.push("handle".to_string());
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                // For byte-slice input parameters, emit the length argument immediately after.
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(format!("(UIntPtr){param_name}.Length"));
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
            // Build all argument parts (including byte-length args)
            let mut arg_parts: Vec<String> = Vec::new();
            if has_receiver {
                arg_parts.push("handle".to_string());
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                arg_parts.push(arg.clone());
                // For byte-slice input parameters, emit the length argument immediately after.
                if matches!(param.ty, TypeRef::Bytes) {
                    arg_parts.push(format!("(UIntPtr){param_name}.Length"));
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
