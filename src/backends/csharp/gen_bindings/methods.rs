//! C# wrapper class and method code generation.

use super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::functions::{is_bytes_result_func, is_bytes_result_method};
use super::{
    StreamingMethodMeta, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, needs_param_teardown, returns_ptr,
};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::doc_emission;
use crate::codegen::generators::trait_bridge::find_bridge_field;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::AdapterConfig;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

/// Skip methods that take opaque handle FFI pointers as first arg but operate on non-opaque types.
/// These are validation/property functions that shouldn't be exposed as static methods.
/// Examples: header_metadata_is_valid, conversion_options_default (Rust naming, snake_case
/// as stored in FunctionDef.name).
fn should_skip_ffi_method(func: &FunctionDef) -> bool {
    let name = &func.name;

    // Skip validation methods (is_valid suffix)
    if name.ends_with("_is_valid") || name == "is_valid" {
        return true;
    }

    // Skip default factory methods (_default suffix from Default::default() impls)
    if name.ends_with("_default") || name == "default" {
        return true;
    }

    false
}

fn sanitize_doc_for_csharp(doc: &str) -> String {
    doc.lines()
        .filter_map(|line| {
            if line.trim().starts_with("use ") && line.contains("::") {
                return None;
            }
            let line = line.replace("`", "");
            let line = line.replace(".unwrap()", "");
            let line = line.replace("```rust", "").replace("```", "");
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate a static wrapper method for a streaming method on an opaque type.
/// Delegates to the instance method on the opaque handle class.
fn gen_opaque_streaming_static_wrapper(
    method: &MethodDef,
    opaque_type_name: &str,
    meta: &StreamingMethodMeta,
    _exception_name: &str,
) -> String {
    let mut out = String::new();

    let class_name = csharp_type_name(opaque_type_name);
    let method_name = to_csharp_name(&method.name);
    let item_type = csharp_type_name(&meta.item_type);

    // Emit XML doc comment
    out.push_str("    /// <summary>\n");
    out.push_str(&format!("    /// {}\n", &method.doc));
    out.push_str("    /// </summary>\n");
    out.push_str(&format!("    /// <param name=\"engine\">Opaque handle to {}</param>\n", opaque_type_name));
    for param in &method.params {
        let param_name = param.name.to_lower_camel_case();
        out.push_str(&format!("    /// <param name=\"{param_name}\"></param>\n"));
    }

    // Static method signature: takes opaque handle + params, returns IAsyncEnumerable<ItemType>
    out.push_str("    public static ");
    if method.is_async {
        out.push_str(&format!("async IAsyncEnumerable<{item_type}> {method_name}Async("));
    } else {
        out.push_str(&format!("IAsyncEnumerable<{item_type}> {method_name}("));
    }

    out.push_str(&format!("{class_name} engine"));
    for param in &method.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        out.push_str(&format!(", {param_type} {param_name}"));
    }
    out.push_str(")\n");
    out.push_str("    {\n");

    // Delegate to the instance method on the engine handle
    if method.is_async {
        out.push_str("        await foreach (var item in engine.");
    } else {
        out.push_str("        foreach (var item in engine.");
    }
    out.push_str(&format!("{method_name}("));
    for (i, param) in method.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let param_name = param.name.to_lower_camel_case();
        out.push_str(&param_name);
    }
    out.push_str("))\n");
    out.push_str("        {\n");
    out.push_str("            yield return item;\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    out
}

/// Generate a wrapper method for a streaming adapter.
/// Emits a public async method that returns IAsyncEnumerable<ItemType>.
fn gen_adapter_wrapper(adapter: &AdapterConfig, _prefix: &str, _exception_name: &str, _api: &ApiSurface) -> String {
    let adapter_name = &adapter.name;
    let method_name = to_csharp_name(adapter_name);
    let Some(owner_type) = adapter.owner_type.as_deref() else {
        return String::new();
    };
    let Some(item_type) = adapter.item_type.as_deref() else {
        return String::new();
    };
    let cs_item_type = csharp_type_name(item_type);
    let owner_cs_name = csharp_type_name(owner_type);

    // Build visible parameters: engine + request params
    let mut param_parts = vec!["IntPtr engine".to_string()];
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type = if param.ty.contains("::") {
            let parts: Vec<&str> = param.ty.split("::").collect();
            csharp_type_name(parts.last().unwrap_or(&"object"))
        } else {
            csharp_type_name(&param.ty)
        };
        param_parts.push(format!("{param_type} {param_name}"));
    }
    let params_decl = param_parts.join(", ");

    // Build setup code to serialize params to JSON and get native handles
    let mut setup_lines = Vec::new();
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type_pascal = to_csharp_name(param.ty.split("::").last().unwrap_or(""));
        setup_lines.push(format!(
            "        var {param_name}Json = JsonSerializer.Serialize({param_name}, JsonSerializationOptions);"
        ));
        setup_lines.push(format!(
            "        var {param_name}Handle = NativeMethods.{param_type_pascal}FromJson({param_name}Json);"
        ));
    }
    let setup_code = if setup_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", setup_lines.join("\n"))
    };

    // Build native call args list (engine + request handles)
    let mut native_args = vec!["engine".to_string()];
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        native_args.push(format!("{param_name}Handle"));
    }
    let native_args_str = native_args.join(", ");

    // Build cleanup code for try-finally
    let mut cleanup_lines = Vec::new();
    for param in &adapter.params {
        let param_name = param.name.to_lower_camel_case();
        let param_type_pascal = to_csharp_name(param.ty.split("::").last().unwrap_or(""));
        cleanup_lines.push(format!(
            "            NativeMethods.{param_type_pascal}Free({param_name}Handle);"
        ));
    }
    let cleanup_code = cleanup_lines.join("\n");

    // Build the iterator handle FFI protocol: {OwnerType}{AdapterName}Start/Next/Free
    let adapter_cs_name = to_csharp_name(adapter_name);
    let start_method = format!("{owner_cs_name}{adapter_cs_name}Start");
    let next_method = format!("{owner_cs_name}{adapter_cs_name}Next");
    let free_method = format!("{owner_cs_name}{adapter_cs_name}Free");

    format!(
        "    public static async IAsyncEnumerable<{cs_item_type}> {method_name}({params_decl})\n    {{\n\
         {setup_code}\
         var iterHandle = NativeMethods.{start_method}({native_args_str});\n\
         if (iterHandle == IntPtr.Zero) throw GetLastError();\n\
         try\n\
         {{\n\
             while (true)\n\
             {{\n\
                 var itemPtr = NativeMethods.{next_method}(iterHandle);\n\
                 if (itemPtr == IntPtr.Zero) break;\n\
                 try\n\
                 {{\n\
                     var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(itemPtr);\n\
                     if (!string.IsNullOrEmpty(json))\n\
                     {{\n\
                         var item = JsonSerializer.Deserialize<{cs_item_type}>(json, JsonOptions)!;\n\
                         yield return item;\n\
                     }}\n\
                 }}\n\
                 finally\n\
                 {{\n\
                     NativeMethods.FreeString(itemPtr);\n\
                 }}\n\
             }}\n\
         }}\n\
         finally\n\
         {{\n\
             NativeMethods.{free_method}(iterHandle);\n\
             {cleanup_code}\n\
         }}\n\
     }}\n\n"
    )
}

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
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    _all_opaque_type_names: &HashSet<String>,
    adapters: &[AdapterConfig],
) -> String {
    use crate::backends::csharp::template_env::render;
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
    let enum_names: HashSet<String> = api.enums.iter().map(|e| csharp_type_name(&e.name)).collect();

    // Truly opaque types (is_opaque = true) — returned/passed as handles, no JSON serialization.
    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Types returned as opaque handles (Named return type from any public function/method).
    let handle_returned_types = super::errors::compute_handle_returned_types(api);

    // Generate wrapper methods for functions.
    // Skip trait-bridge `clear_fn` functions: their registry-clearing API is exposed via the
    // generated `{Trait}Registry.Clear()` helper in TraitBridges.cs, and the FFI layer exports
    // no regular `{prefix}_{clear_fn}` symbol for them. Emitting a wrapper here would call a
    // non-existent NativeMethods entry point.
    for func in api.functions.iter().filter(|f| {
        !exclude_functions.contains(&f.name)
            && !should_skip_ffi_method(f)
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, trait_bridges)
    }) {
        // Check if this function has a bridge_field binding (e.g., visitor field on options)
        let bridge_field = find_bridge_field(func, &api.types, trait_bridges);
        if let Some(bm) = bridge_field {
            out.push_str(&gen_bridge_field_wrapper_function(
                func,
                &bm,
                exception_name,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
            ));
        } else {
            out.push_str(&gen_wrapper_function(
                func,
                exception_name,
                prefix,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
                bridge_param_names,
                bridge_type_aliases,
                has_visitor_callbacks,
                &api.types,
            ));
        }
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
            // Skip Rust-ism methods: is_valid (move to instance method on type) and default
            // (idiomatic C# uses parameterless constructor or field defaults).
            if method.name == "is_valid" || method.name == "default" {
                continue;
            }
            out.push_str(&gen_wrapper_method(
                method,
                exception_name,
                prefix,
                &typ.name,
                &enum_names,
                &true_opaque_types,
                &handle_returned_types,
                bridge_param_names,
                bridge_type_aliases,
                &api.types,
            ));
        }
    }

    // Emit adapter wrapper methods for streaming adapters
    for adapter in adapters {
        if matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
            out.push_str(&gen_adapter_wrapper(adapter, prefix, exception_name, api));
        }
    }

    // Emit Register* and Unregister* facade methods for trait bridges.
    // Bridge factory returns an IntPtr handle; the facade completes the registration.
    for bridge_cfg in trait_bridges {
        let trait_pascal = csharp_type_name(&bridge_cfg.trait_name);
        let has_super = bridge_cfg.super_trait.is_some();

        // Register{TraitName} — takes IntPtr handle from bridge factory, completes registration
        let register_method_name = format!("Register{trait_pascal}");
        out.push_str(&format!(
            "    /// <summary>Complete native registration of a {trait_pascal} implementation</summary>\n"
        ));
        out.push_str(&format!(
            "    public static void {}(IntPtr handle)\n",
            register_method_name
        ));
        out.push_str("    {\n");
        out.push_str("        if (handle == IntPtr.Zero) throw new ArgumentException(\"handle is null\");\n");
        out.push_str(&format!(
            "        var bridge = GCHandle.FromIntPtr(handle).Target as {}Bridge ?? throw new InvalidOperationException(\"Invalid bridge handle\");\n",
            trait_pascal
        ));
        out.push_str(&format!(
            "        var impl = bridge.GetType().GetField(\"_impl\", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance)?.GetValue(bridge) as I{};\n",
            trait_pascal
        ));
        out.push_str(
            "        if (impl == null) throw new InvalidOperationException(\"Cannot extract impl from bridge\");\n",
        );
        if has_super {
            out.push_str("        var name = impl.Name;\n");
        } else {
            out.push_str("        var name = System.Guid.NewGuid().ToString();\n");
        }
        out.push_str(&format!(
            "        var ec = NativeMethods.Register{}(name, bridge._vtable, handle, out var outError);\n",
            trait_pascal
        ));
        out.push_str("        if (ec != 0) {\n");
        out.push_str("            var msg = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(outError) ?? \"Register failed\";\n");
        out.push_str(&format!("            throw new {exception_name}(ec, msg);\n"));
        out.push_str("        }\n");
        out.push_str("    }\n\n");

        // Unregister{TraitName} — only if unregister_fn is configured
        if bridge_cfg.unregister_fn.is_some() {
            let unregister_method_name = format!("Unregister{trait_pascal}");
            out.push_str(&format!(
                "    /// <summary>Unregister a {trait_pascal} implementation by name</summary>\n"
            ));
            out.push_str(&format!(
                "    public static void {}(string name)\n",
                unregister_method_name
            ));
            out.push_str("    {\n");
            out.push_str("        ArgumentNullException.ThrowIfNull(name);\n");
            out.push_str(&format!(
                "        var ec = NativeMethods.Unregister{}(name, out var outError);\n",
                trait_pascal
            ));
            out.push_str("        if (ec != 0) {\n");
            out.push_str("            var msg = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(outError) ?? \"Unregister failed\";\n");
            out.push_str(&format!("            throw new {exception_name}(ec, msg);\n"));
            out.push_str("        }\n");
            out.push_str("    }\n\n");
        }
    }

    // Emit Clear* facade methods for trait bridges with clear_fn configured.
    // These static methods forward to the {Trait}Registry.Clear() methods.
    // The method name is derived from clear_fn (e.g., "clear_ocr_backends" → "ClearOcrBackends"),
    // not from the trait name, to match the Rust FFI free-function naming convention.
    for bridge_cfg in trait_bridges {
        if let Some(clear_fn) = &bridge_cfg.clear_fn {
            let trait_pascal = csharp_type_name(&bridge_cfg.trait_name);
            let clear_method_name = to_csharp_name(clear_fn);

            out.push_str(&format!(
                "    /// <summary>Clear all registered {} implementations</summary>\n",
                trait_pascal
            ));
            out.push_str(&format!("    public static void {}()\n", clear_method_name));
            out.push_str("    {\n");
            out.push_str(&format!("        {}Registry.Clear();\n", trait_pascal));
            out.push_str("    }\n\n");
        }
    }

    // Add error handling helper — dispatches typed exceptions by error code
    let has_base_error = !api.errors.is_empty();
    let (base_exception_class, has_invalid_input_variant, variant_dispatch_lines) = if has_base_error {
        let base_error = &api.errors[0];
        let base_ex = format!("{}Exception", base_error.name);
        let has_invalid = base_error.variants.iter().any(|v| v.name == "InvalidInput");
        // Build per-variant message-prefix dispatch. Each thiserror Display template starts
        // with a literal prefix (e.g. `"not_found: {0}"`), giving the runtime message a stable
        // prefix the binding can match on. Skip the InvalidInput variant — that one is dispatched
        // via the explicit `code == 1` arm above. Order by descending prefix length so that
        // overlapping prefixes (e.g. `"forbidden: waf/blocked: "` vs `"forbidden: "`) match the
        // longer one first.
        let mut variants_with_prefix: Vec<(String, String)> = base_error
            .variants
            .iter()
            .filter(|v| v.name != "InvalidInput")
            .filter_map(|v| {
                let template = v.message_template.as_deref()?;
                let prefix_end = template.find('{').unwrap_or(template.len());
                let prefix = template[..prefix_end].trim_end().to_string();
                if prefix.is_empty() {
                    return None;
                }
                Some((format!("{}Exception", v.name), prefix))
            })
            .collect();
        // Longest prefix first so e.g. "forbidden: waf/blocked: " wins over "forbidden: ".
        variants_with_prefix.sort_by_key(|item| std::cmp::Reverse(item.1.len()));
        let dispatch_lines: Vec<String> = variants_with_prefix
            .into_iter()
            .map(|(class, prefix)| {
                let escaped_prefix = prefix.replace('\\', "\\\\").replace('"', "\\\"");
                format!("        if (message.StartsWith(\"{escaped_prefix}\")) return new {class}(message);")
            })
            .collect();
        (base_ex, has_invalid, dispatch_lines)
    } else {
        (String::new(), false, Vec::new())
    };

    out.push_str(&render(
        "error_helper_method.jinja",
        Value::from_serialize(serde_json::json!({
            "exception_name": exception_name,
            "has_base_error": has_base_error,
            "base_exception_class": base_exception_class,
            "has_invalid_input_variant": has_invalid_input_variant,
            "variant_dispatch_lines": variant_dispatch_lines,
        })),
    ));

    out.push_str("}\n");

    out
}

#[allow(clippy::too_many_arguments)]
fn gen_wrapper_function(
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
    for param in &visible_params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
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
                out.push_str(&format!(
                    "{body_indent}if (nativeResult == IntPtr.Zero)\n{body_indent}{{\n{body_indent}    return null;\n{body_indent}}}\n",
                ));
            } else {
                out.push_str(&format!(
                    "{body_indent}if (nativeResult == IntPtr.Zero)\n{body_indent}{{\n{body_indent}    throw GetLastError();\n{body_indent}}}\n",
                ));
            }
        } else if func.error_type.is_some() {
            out.push_str(&format!(
                "{body_indent}if (NativeMethods.LastErrorCode() != 0)\n{body_indent}{{\n{body_indent}    throw GetLastError();\n{body_indent}}}\n",
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
fn gen_bridge_field_wrapper_function(
    func: &FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    exception_name: &str,
    _enum_names: &HashSet<String>,
    _true_opaque_types: &HashSet<String>,
    _handle_returned_types: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    // Visible params (bridge field is embedded in the options param)
    let visible_params: Vec<crate::core::ir::ParamDef> = func.params.to_vec();

    // XML doc comment
    doc_emission::emit_csharp_doc(&mut out, &func.doc, "    ", exception_name);
    for param in &visible_params {
        if !func.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&format!(
                "    /// <param name=\"{param_name}\">{optional_text}</param>\n"
            ));
        }
    }

    out.push_str("    public static ");

    // Return type
    if func.is_async {
        if func.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            let return_type = csharp_type(&func.return_type);
            out.push_str(&format!("async Task<{return_type}>"));
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

    // Parameters (all visible, since bridge is embedded in options param)
    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(&format!("{param_type}? {param_name}"));
        } else {
            out.push_str(&format!("{param_type} {param_name}"));
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }
    out.push_str(")\n    {\n");

    // Extract the bridge field value and options parameter name
    let options_param = &bridge_match.param_name;
    let options_param_camel = options_param.to_lower_camel_case();
    let field_name = &bridge_match.field_name;
    let field_name_pascal = to_csharp_name(field_name);
    let trait_pascal = csharp_type_name(&bridge_match.bridge.trait_name);
    let options_pascal = csharp_type_name(&bridge_match.options_type);
    let result_pascal = match &func.return_type {
        TypeRef::Named(name) => csharp_type_name(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => csharp_type_name(name),
            _ => csharp_type(&func.return_type).into_owned(),
        },
        _ => csharp_type(&func.return_type).into_owned(),
    };

    // Extract bridge from options (must be optional)
    out.push_str(&format!(
        "        var {field_name} = {options_param_camel}?.{field_name_pascal};\n"
    ));

    // Serialize options to JSON (excluding bridge field via JsonIgnore)
    out.push_str(&format!(
        "        var {options_param_camel}Json = {options_param_camel} != null ? JsonSerializer.Serialize({options_param_camel}, JsonSerializationOptions) : \"null\";\n"
    ));

    // Deserialize into native options handle
    out.push_str(&format!(
        "        var {options_param_camel}Handle = NativeMethods.{options_pascal}FromJson({options_param_camel}Json);\n"
    ));

    // If-bridge-present logic
    out.push_str(&format!(
        "        try\n        {{\n            if ({field_name} != null)\n            {{\n"
    ));
    out.push_str(&format!(
        "                using var bridge = new {trait_pascal}Bridge({field_name});\n"
    ));
    // Insert bridge into the static registry so FFI callbacks (which receive
    // bridge._bridgeId as userData) can dispatch back to the managed visitor.
    out.push_str(&format!("                lock ({trait_pascal}Bridge._registryLock)\n"));
    out.push_str("                {\n");
    out.push_str(&format!(
        "                    {trait_pascal}Bridge._bridgeRegistry[bridge._bridgeId] = bridge;\n"
    ));
    out.push_str("                }\n");
    out.push_str(&format!(
        "                var bridgeHandle = NativeMethods.{trait_pascal}BridgeNew(bridge._vtable, bridge._bridgeId);\n"
    ));
    out.push_str("                if (bridgeHandle == IntPtr.Zero) throw GetLastError();\n");
    out.push_str("                try\n                {\n");

    // Call native function with injected bridge
    let cs_native_name = to_csharp_name(&func.name);
    out.push_str(&format!(
        "                    NativeMethods.{options_pascal}Set{field_name_pascal}({options_param_camel}Handle, bridgeHandle);\n"
    ));

    // Build the native call
    if func.return_type != TypeRef::Unit {
        out.push_str("                    var nativeResult = ");
    } else {
        out.push_str("                    ");
    }

    out.push_str(&format!("NativeMethods.{cs_native_name}("));

    // Build call args (non-options params for convert, or all params if not convert-like)
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                format!("{options_param_camel}Handle")
            } else {
                p.name.to_lower_camel_case().to_string()
            }
        })
        .collect();

    for (i, arg) in call_args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg);
    }
    out.push_str(");\n");

    // Error check
    if func.return_type != TypeRef::Unit {
        out.push_str("                    if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
    }

    // Handle return value through the generated FFI JSON helpers for the actual return type.
    if func.return_type != TypeRef::Unit {
        out.push_str(&format!(
            "                    var jsonPtr = NativeMethods.{result_pascal}ToJson(nativeResult);\n"
        ));
        out.push_str(
            "                    var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n",
        );
        out.push_str("                    NativeMethods.FreeString(jsonPtr);\n");
        out.push_str(&format!(
            "                    NativeMethods.{result_pascal}Free(nativeResult);\n"
        ));
        out.push_str(&format!(
            "                    return JsonSerializer.Deserialize<{result_pascal}>(json ?? \"null\", JsonOptions)!;\n"
        ));
    }

    out.push_str("                }\n");
    out.push_str("                finally\n");
    out.push_str("                {\n");
    out.push_str(&format!(
        "                    NativeMethods.{trait_pascal}BridgeFree(bridgeHandle);\n"
    ));
    // Remove registry entry now that Rust will not call back again.
    out.push_str(&format!(
        "                    lock ({trait_pascal}Bridge._registryLock)\n"
    ));
    out.push_str("                    {\n");
    out.push_str(&format!(
        "                        {trait_pascal}Bridge._bridgeRegistry.Remove(bridge._bridgeId);\n"
    ));
    out.push_str("                    }\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("            else\n");
    out.push_str("            {\n");

    // Call without bridge
    if func.return_type != TypeRef::Unit {
        out.push_str("                var nativeResult = ");
    } else {
        out.push_str("                ");
    }

    out.push_str(&format!("NativeMethods.{cs_native_name}("));
    for (i, arg) in call_args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg);
    }
    out.push_str(");\n");

    if func.return_type != TypeRef::Unit {
        out.push_str("                if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
        out.push_str(&format!(
            "                var jsonPtr = NativeMethods.{result_pascal}ToJson(nativeResult);\n"
        ));
        out.push_str(
            "                var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n",
        );
        out.push_str("                NativeMethods.FreeString(jsonPtr);\n");
        out.push_str(&format!(
            "                NativeMethods.{result_pascal}Free(nativeResult);\n"
        ));
        out.push_str(&format!(
            "                return JsonSerializer.Deserialize<{result_pascal}>(json ?? \"null\", JsonOptions)!;\n"
        ));
    }

    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        finally\n");
    out.push_str("        {\n");
    out.push_str(&format!(
        "            NativeMethods.{options_pascal}Free({options_param_camel}Handle);\n"
    ));
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    out
}

#[allow(clippy::too_many_arguments)]
fn gen_wrapper_method(
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
    for param in &visible_params {
        if !param.optional && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
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
