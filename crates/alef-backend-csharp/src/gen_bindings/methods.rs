//! C# wrapper class and method code generation.

use super::errors::{
    emit_return_marshalling, emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented,
};
use super::functions::{is_bytes_result_func, is_bytes_result_method};
use super::{
    StreamingMethodMeta, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, returns_ptr,
};
use crate::type_map::csharp_type;
use alef_codegen::doc_emission;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase};
use std::collections::{HashMap, HashSet};

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_wrapper_class(
    api: &ApiSurface,
    namespace: &str,
    class_name: &str,
    exception_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
    streaming_methods: &HashSet<String>,
    _streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    exclude_functions: &HashSet<String>,
) -> String {
    use crate::template_env::render;
    use minijinja::Value;

    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().flat_map(|t| t.methods.iter()).any(|m| m.is_async);

    let mut out = render(
        "wrapper_class_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "has_async": has_async,
        })),
    );
    out.push('\n');

    // Enum names: used to distinguish opaque struct handles from enum return types.
    let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.to_pascal_case()).collect();

    // Truly opaque types (is_opaque = true) — returned/passed as handles, no JSON serialization.
    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Generate wrapper methods for functions
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        out.push_str(&gen_wrapper_function(
            func,
            exception_name,
            prefix,
            &enum_names,
            &true_opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_callbacks,
        ));
    }

    // Generate wrapper methods for type methods (prefixed with type name to avoid collisions).
    // Skip streaming adapter methods — their FFI signature uses callbacks that P/Invoke can't call.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        // Skip opaque types — their methods belong on the opaque handle class, not the static wrapper
        if typ.is_opaque {
            continue;
        }
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
            out.push_str(&gen_wrapper_method(
                method,
                exception_name,
                prefix,
                &typ.name,
                &enum_names,
                &true_opaque_types,
                bridge_param_names,
                bridge_type_aliases,
            ));
        }
    }

    // Add error handling helper — dispatches typed exceptions by error code
    let has_base_error = !api.errors.is_empty();
    let (base_exception_class, has_invalid_input_variant) = if has_base_error {
        let base_error = &api.errors[0];
        let base_ex = format!("{}Exception", base_error.name.to_pascal_case());
        let has_invalid = base_error
            .variants
            .iter()
            .any(|v| v.name.to_pascal_case() == "InvalidInput");
        (base_ex, has_invalid)
    } else {
        (String::new(), false)
    };

    out.push_str(&render(
        "error_helper_method.jinja",
        Value::from_serialize(serde_json::json!({
            "exception_name": exception_name,
            "has_base_error": has_base_error,
            "base_exception_class": base_exception_class,
            "has_invalid_input_variant": has_invalid_input_variant,
        })),
    ));

    out.push_str("}\n");

    out
}

#[allow(clippy::too_many_arguments)]
fn gen_wrapper_function(
    func: &FunctionDef,
    _exception_name: &str,
    _prefix: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
) -> String {
    use crate::template_env::render;

    let mut out = String::with_capacity(1024);

    // Collect visible params (non-bridge) for the public C# signature.
    let visible_params: Vec<alef_core::ir::ParamDef> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    // XML doc comment using shared doc emission
    doc_emission::emit_csharp_doc(&mut out, &func.doc, "    ");
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
    out.push_str(&to_csharp_name(&func.name));
    out.push('(');

    // Parameters (bridge params stripped from public signature)
    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        // Config parameters are optional in practice (callers often omit them and expect defaults)
        let is_optional_by_convention = param.name == "config" && matches!(param.ty, TypeRef::Named(_));
        if (param.optional || is_optional_by_convention) && !param_type.ends_with('?') {
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

    // Null checks for required string/object parameters
    // Skip config parameters — they are optional by convention and will be defaulted
    for param in &visible_params {
        let is_optional_by_convention = param.name == "config" && matches!(param.ty, TypeRef::Named(_));
        if !param.optional
            && !is_optional_by_convention
            && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes)
        {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    // Result<Vec<u8>> uses the out-param convention — emit specialized body and return early.
    if is_bytes_result_func(func) {
        let cs_native_name = to_csharp_name(&func.name);
        // Emit setup for Named and Bytes parameters before calling the native method
        emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);
        // Build the args block for the template: each arg on its own indented line with trailing comma.
        let mut args_block = String::new();
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&format!("            {arg},\n"));
            // For byte-slice input parameters, emit the length argument immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&format!("            (UIntPtr){param_name}.Length,\n"));
            }
        }
        // Build cleanup block for try-finally
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(&mut cleanup_block, &visible_params, "            ", true_opaque_types);
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

    // Detect if this is the main convert function with visitor support.
    // The convert function should have (string, ConversionOptions?) signature and has_visitor_callbacks=true.
    let has_options_param = visible_params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if n == "ConversionOptions"));
    let is_convert_with_visitor = has_visitor_callbacks && func.name == "convert" && has_options_param;

    // Special handling for convert function with visitor support:
    // Extract visitor from options before serialization
    if is_convert_with_visitor {
        out.push_str("        var visitor = options?.Visitor;\n");
        out.push_str(
            "        var optionsJson = options != null ? JsonSerializer.Serialize(options, JsonOptions) : \"null\";\n",
        );
        out.push_str("        var optionsHandle = NativeMethods.ConversionOptionsFromJson(optionsJson);\n");
        out.push_str("        try\n");
        out.push_str("        {\n");
        out.push_str("            if (visitor != null)\n");
        out.push_str("            {\n");
        out.push_str("                using var bridge = new HtmlVisitorBridge(visitor);\n");
        out.push_str(
            "                var bridgeHandle = NativeMethods.HtmlVisitorBridgeNew(bridge._vtable, IntPtr.Zero);\n",
        );
        out.push_str("                if (bridgeHandle == IntPtr.Zero) throw GetLastError();\n");
        out.push_str("                try\n");
        out.push_str("                {\n");
        out.push_str("                    NativeMethods.ConversionOptionsSetVisitor(optionsHandle, bridgeHandle);\n");
        out.push_str("                    var nativeResult = NativeMethods.Convert(html, optionsHandle);\n");
        out.push_str("                    if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
        out.push_str("                    var jsonPtr = NativeMethods.ConversionResultToJson(nativeResult);\n");
        out.push_str("                    var json = Marshal.PtrToStringUTF8(jsonPtr);\n");
        out.push_str("                    NativeMethods.FreeString(jsonPtr);\n");
        out.push_str("                    NativeMethods.ConversionResultFree(nativeResult);\n");
        out.push_str("                    return JsonSerializer.Deserialize<ConversionResult>(json ?? \"null\", JsonOptions)!;\n");
        out.push_str("                }\n");
        out.push_str("                finally\n");
        out.push_str("                {\n");
        out.push_str("                    NativeMethods.HtmlVisitorBridgeFree(bridgeHandle);\n");
        out.push_str("                }\n");
        out.push_str("            }\n");
        out.push_str("            else\n");
        out.push_str("            {\n");
        out.push_str("                var nativeResult = NativeMethods.Convert(html, optionsHandle);\n");
        out.push_str("                if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
        out.push_str("                var jsonPtr = NativeMethods.ConversionResultToJson(nativeResult);\n");
        out.push_str("                var json = Marshal.PtrToStringUTF8(jsonPtr);\n");
        out.push_str("                NativeMethods.FreeString(jsonPtr);\n");
        out.push_str("                NativeMethods.ConversionResultFree(nativeResult);\n");
        out.push_str(
            "                return JsonSerializer.Deserialize<ConversionResult>(json ?? \"null\", JsonOptions)!;\n",
        );
        out.push_str("            }\n");
        out.push_str("        }\n");
        out.push_str("        finally\n");
        out.push_str("        {\n");
        out.push_str("            NativeMethods.ConversionOptionsFree(optionsHandle);\n");
        out.push_str("        }\n");
        out.push_str("    }\n\n");
        return out;
    }

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);

    // Method body - delegation to native method with proper marshalling
    let cs_native_name = to_csharp_name(&func.name);

    if func.is_async {
        // Async: wrap in Task.Run for non-blocking execution. CS1997 disallows
        // `return await Task.Run(...)` in an `async Task` (non-generic) method,
        // so for unit returns we drop the `return`.
        if func.return_type == TypeRef::Unit {
            out.push_str("        await Task.Run(() =>\n        {\n");
        } else {
            out.push_str("        return await Task.Run(() =>\n        {\n");
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
                out.push_str(render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if i < arg_parts.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        // Check for FFI error (null result means the call failed).
        // For Optional(_) return types, null means None (not found), not an error.
        if func.return_type != TypeRef::Unit {
            if matches!(func.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "            if (nativeResult == IntPtr.Zero)\n            {\n                return null;\n            }\n",
                );
            } else {
                out.push_str(
                    "            if (nativeResult == IntPtr.Zero)\n            {\n                throw GetLastError();\n            }\n",
                );
            }
        }

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            "            ",
            enum_names,
            true_opaque_types,
        );
        emit_named_param_teardown_indented(&mut out, &visible_params, "            ", true_opaque_types);
        emit_return_statement_indented(&mut out, &func.return_type, "            ");
        out.push_str("        });\n");
    } else {
        if func.return_type != TypeRef::Unit {
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
            out.push_str("        );\n");
        }

        // Check for FFI error (null result means the call failed).
        // Only emit for pointer-returning functions — numeric returns (ulong, uint, bool)
        // don't use IntPtr.Zero as an error sentinel.
        // For Optional(_) return types, null means None (not found), not an error.
        if func.return_type != TypeRef::Unit && returns_ptr(&func.return_type) {
            if matches!(func.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "        if (nativeResult == IntPtr.Zero)\n        {\n            return null;\n        }\n",
                );
            } else {
                out.push_str(
                    "        if (nativeResult == IntPtr.Zero)\n        {\n            throw GetLastError();\n        }\n",
                );
            }
        }

        emit_return_marshalling(&mut out, &func.return_type, enum_names, true_opaque_types);
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types);
        emit_return_statement(&mut out, &func.return_type);
    }

    out.push_str("    }\n\n");

    out
}

#[allow(clippy::too_many_arguments)]
fn gen_wrapper_method(
    method: &MethodDef,
    _exception_name: &str,
    _prefix: &str,
    type_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    use crate::template_env::render;

    let mut out = String::with_capacity(1024);

    // Collect visible params (non-bridge) for the public C# signature.
    let visible_params: Vec<alef_core::ir::ParamDef> = method
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    // XML doc comment using shared doc emission
    doc_emission::emit_csharp_doc(&mut out, &method.doc, "    ");
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
    let method_cs_name = format!("{}{}", type_name, to_csharp_name(&method.name));
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
        // Config parameters are optional in practice (callers often omit them and expect defaults)
        let is_optional_by_convention = param.name == "config" && matches!(param.ty, TypeRef::Named(_));
        if (param.optional || is_optional_by_convention) && !param_type.ends_with('?') {
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

    // Null checks for required string/object parameters
    // Skip config parameters — they are optional by convention and will be defaulted
    for param in &visible_params {
        let is_optional_by_convention = param.name == "config" && matches!(param.ty, TypeRef::Named(_));
        if !param.optional
            && !is_optional_by_convention
            && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes)
        {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", minijinja::context! { param_name }));
        }
    }

    let cs_native_name = format!("{}{}", type_name.to_pascal_case(), to_csharp_name(&method.name));

    // Result<Vec<u8>> uses the out-param convention — emit specialized body and return early.
    if is_bytes_result_method(method) {
        // Emit setup for Named and Bytes parameters before calling the native method
        emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);
        // Build the args block: receiver (if any) then visible params, each with trailing comma.
        let mut args_block = String::new();
        if has_receiver {
            args_block.push_str("            handle,\n");
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&format!("            {arg},\n"));
            // For byte-slice input parameters, emit the length argument immediately after.
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&format!("            (UIntPtr){param_name}.Length,\n"));
            }
        }
        // Build cleanup block for try-finally
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(&mut cleanup_block, &visible_params, "            ", true_opaque_types);
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
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);

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

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "            ",
            enum_names,
            true_opaque_types,
        );
        emit_named_param_teardown_indented(&mut out, &visible_params, "            ", true_opaque_types);
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

        emit_return_marshalling(&mut out, &method.return_type, enum_names, true_opaque_types);
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n\n");

    out
}
