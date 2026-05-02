//! C# wrapper class and method code generation.

use super::errors::{
    emit_return_marshalling, emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented,
};
use super::{
    csharp_file_header, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, returns_ptr,
};
use crate::type_map::csharp_type;
use alef_codegen::doc_emission;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase};
use std::collections::HashSet;

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
    exclude_functions: &HashSet<String>,
) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Runtime.InteropServices;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n");
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().flat_map(|t| t.methods.iter()).any(|m| m.is_async);
    if has_async {
        out.push_str("using System.Threading.Tasks;\n");
    }
    out.push('\n');

    out.push_str(&format!("namespace {};\n\n", namespace));

    out.push_str(&format!("public static class {}\n", class_name));
    out.push_str("{\n");
    out.push_str("    private static readonly JsonSerializerOptions JsonOptions = new()\n");
    out.push_str("    {\n");
    out.push_str("        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },\n");
    out.push_str("        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingDefault\n");
    out.push_str("    };\n\n");

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

    // Inject ConvertWithVisitor when a visitor bridge is configured.
    if has_visitor_callbacks {
        out.push_str(&crate::gen_visitor::gen_convert_with_visitor_method(
            exception_name,
            prefix,
        ));
    }

    // Add error handling helper — dispatches typed exceptions by error code
    out.push_str("    private static Exception GetLastError()\n");
    out.push_str("    {\n");
    out.push_str("        var code = NativeMethods.LastErrorCode();\n");
    out.push_str("        var ctxPtr = NativeMethods.LastErrorContext();\n");
    out.push_str("        var message = Marshal.PtrToStringUTF8(ctxPtr) ?? \"Unknown error\";\n");
    // Dispatch typed exceptions: code 1 → InvalidInputException (if present in IR errors),
    // code 2 → base error exception class, fallback → generic exception with code.
    if !api.errors.is_empty() {
        let base_error = &api.errors[0];
        let base_ex = format!("{}Exception", base_error.name.to_pascal_case());
        let has_invalid_input = base_error
            .variants
            .iter()
            .any(|v| v.name.to_pascal_case() == "InvalidInput");
        if has_invalid_input {
            out.push_str("        if (code == 1) return new InvalidInputException(message);\n");
        }
        out.push_str(&format!("        if (code == 2) return new {base_ex}(message);\n"));
    }
    out.push_str(&format!("        return new {}(code, message);\n", exception_name));
    out.push_str("    }\n");

    out.push_str("}\n");

    out
}

fn gen_wrapper_function(
    func: &FunctionDef,
    _exception_name: &str,
    _prefix: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
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
            out.push_str(&format!(
                "    /// <param name=\"{}\">{}</param>\n",
                param.name.to_lower_camel_case(),
                if param.optional { "Optional." } else { "" }
            ));
        }
    }

    out.push_str("    public static ");

    // Return type — use async Task<T> for async methods
    if func.is_async {
        if func.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            out.push_str(&format!("async Task<{}>", csharp_type(&func.return_type)));
        }
    } else if func.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&func.return_type));
    }

    out.push_str(&format!(" {}", to_csharp_name(&func.name)));
    out.push('(');

    // Parameters (bridge params stripped from public signature)
    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let mapped = csharp_type(&param.ty);
        if param.optional && !mapped.ends_with('?') {
            out.push_str(&format!("{mapped}? {param_name}"));
        } else {
            out.push_str(&format!("{mapped} {param_name}"));
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    // Null checks for required string/object parameters
    for param in &visible_params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("        ArgumentNullException.ThrowIfNull({param_name});\n"));
        }
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

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("                {arg}"));
                if i < visible_params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("            );\n");
        }

        // Check for FFI error (null result means the call failed).
        if func.return_type != TypeRef::Unit {
            out.push_str(
                "            if (nativeResult == IntPtr.Zero)\n            {\n                throw GetLastError();\n            }\n",
            );
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

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("            {arg}"));
                if i < visible_params.len() - 1 {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("        );\n");
        }

        // Check for FFI error (null result means the call failed).
        // Only emit for pointer-returning functions — numeric returns (ulong, uint, bool)
        // don't use IntPtr.Zero as an error sentinel.
        if func.return_type != TypeRef::Unit && returns_ptr(&func.return_type) {
            out.push_str(
                "        if (nativeResult == IntPtr.Zero)\n        {\n            throw GetLastError();\n        }\n",
            );
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
            out.push_str(&format!(
                "    /// <param name=\"{}\">{}</param>\n",
                param.name.to_lower_camel_case(),
                if param.optional { "Optional." } else { "" }
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
            out.push_str(&format!("async Task<{}>", csharp_type(&method.return_type)));
        }
    } else if method.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&method.return_type));
    }

    // Prefix method name with type name to avoid collisions (e.g., MetadataConfigDefault)
    let method_cs_name = format!("{}{}", type_name, to_csharp_name(&method.name));
    out.push_str(&format!(" {method_cs_name}"));
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
        let mapped = csharp_type(&param.ty);
        if param.optional && !mapped.ends_with('?') {
            out.push_str(&format!("{mapped}? {param_name}"));
        } else {
            out.push_str(&format!("{mapped} {param_name}"));
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    // Null checks for required string/object parameters
    for param in &visible_params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&format!("        ArgumentNullException.ThrowIfNull({param_name});\n"));
        }
    }

    // Serialize Named (opaque handle) params to JSON and obtain native handles.
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);

    // Method body - delegation to native method with proper marshalling.
    // Use the type-prefixed name to match the P/Invoke declaration, which includes the type
    // name to avoid collisions between different types with identically-named methods
    // (e.g. BrowserConfig::default and CrawlConfig::default).
    let cs_native_name = format!("{}{}", type_name.to_pascal_case(), to_csharp_name(&method.name));

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

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if !has_receiver && visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let total = if has_receiver {
                visible_params.len() + 1
            } else {
                visible_params.len()
            };
            let mut idx = 0usize;
            if has_receiver {
                out.push_str("                handle");
                if total > 1 {
                    out.push(',');
                }
                out.push('\n');
                idx += 1;
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("                {arg}"));
                if idx < total - 1 {
                    out.push(',');
                }
                out.push('\n');
                idx += 1;
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

        out.push_str(&format!("NativeMethods.{}(", cs_native_name));

        if !has_receiver && visible_params.is_empty() {
            out.push_str(");\n");
        } else {
            out.push('\n');
            let total = if has_receiver {
                visible_params.len() + 1
            } else {
                visible_params.len()
            };
            let mut idx = 0usize;
            if has_receiver {
                out.push_str("            handle");
                if total > 1 {
                    out.push(',');
                }
                out.push('\n');
                idx += 1;
            }
            for param in visible_params.iter() {
                let param_name = param.name.to_lower_camel_case();
                let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!("            {arg}"));
                if idx < total - 1 {
                    out.push(',');
                }
                out.push('\n');
                idx += 1;
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
