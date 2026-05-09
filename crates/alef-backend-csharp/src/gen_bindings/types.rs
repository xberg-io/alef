//! C# opaque handle and record type code generation.

use super::errors::{
    emit_return_marshalling, emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented,
};
use super::{
    StreamingMethodMeta, csharp_file_header, emit_named_param_setup, emit_named_param_teardown,
    emit_named_param_teardown_indented, is_tuple_field, returns_ptr,
};
use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::{DefaultValue, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase};
use std::collections::{HashMap, HashSet};

pub(super) fn gen_opaque_handle(
    typ: &TypeDef,
    namespace: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    all_opaque_type_names: &HashSet<String>,
) -> String {
    use crate::template_env::render;
    use minijinja::Value;

    // Determine which additional using directives are needed.
    // Streaming methods reuse the JSON / async / generics / pinvoke machinery, so they count
    // toward `has_methods`, `needs_async`, and `needs_list` regardless of the non-streaming set.
    let has_streaming = typ
        .methods
        .iter()
        .any(|m| streaming_methods.contains(&m.name) && streaming_methods_meta.contains_key(&m.name));
    let has_methods = has_streaming || typ.methods.iter().any(|m| !streaming_methods.contains(&m.name));
    let uses_list = |tr: &TypeRef| -> bool {
        matches!(tr, TypeRef::Vec(_))
            || matches!(tr, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)))
    };
    let needs_list = has_streaming
        || (has_methods
            && typ
                .methods
                .iter()
                .any(|m| uses_list(&m.return_type) || m.params.iter().any(|p| uses_list(&p.ty))));
    let needs_async = has_streaming
        || (has_methods
            && typ
                .methods
                .iter()
                .any(|m| m.is_async && !streaming_methods.contains(&m.name)));

    let class_name = typ.name.to_pascal_case();
    let free_method = format!("{}Free", class_name);

    // Prepare doc lines if present
    let doc_lines: Vec<String> = if !typ.doc.is_empty() {
        typ.doc.lines().map(|l| l.to_string()).collect()
    } else {
        vec![]
    };

    let mut out = render(
        "opaque_handle_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "free_method": free_method,
            "has_methods": has_methods,
            "needs_list": needs_list,
            "needs_async": needs_async,
            "needs_streaming": has_streaming,
            "doc": !typ.doc.is_empty(),
            "doc_lines": doc_lines,
        })),
    );
    out.push('\n');

    // Generate public methods for each non-streaming method on this opaque type.
    // These delegate to NativeMethods using this.Handle as the receiver.
    // Use the full set of opaque type names so that methods returning other opaque
    // types (e.g., LanguageRegistry::get_language → Language) are wrapped directly
    // as `new Language(ptr)` rather than being incorrectly JSON-serialized.
    let true_opaque_types = all_opaque_type_names;
    for method in &typ.methods {
        if streaming_methods.contains(&method.name) {
            if let Some(meta) = streaming_methods_meta.get(&method.name) {
                out.push('\n');
                out.push_str(&gen_opaque_streaming_method(
                    method,
                    &class_name,
                    exception_name,
                    meta,
                ));
            }
            continue;
        }
        out.push('\n');
        out.push_str(&gen_opaque_method(
            method,
            &class_name,
            exception_name,
            enum_names,
            true_opaque_types,
        ));
    }

    out.push_str("}\n");

    out
}

/// Generate a streaming method on an opaque handle class as `IAsyncEnumerable<Item>` driven by
/// the FFI iterator-handle protocol (`{type}{method}Start` / `Next` / `Free`).
///
/// The body:
/// 1. Serializes the request to JSON and obtains a `{Request}FromJson` handle.
/// 2. Calls `{type}{method}Start(this.Handle, requestHandle)` to obtain the stream handle.
/// 3. In a `try` block, repeatedly calls `{type}{method}Next(streamHandle)`:
///    - Non-null pointer → deserialize chunk via `{Item}ToJson` + `JsonSerializer`, yield it.
///    - Null pointer → check `LastErrorCode()`: 0 = clean end-of-stream, non-zero = error.
/// 4. In `finally`, frees both the stream handle and the request handle.
fn gen_opaque_streaming_method(
    method: &MethodDef,
    class_name: &str,
    exception_name: &str,
    meta: &StreamingMethodMeta,
) -> String {
    let cs_method_name = to_csharp_name(&method.name);
    let cs_type_name = class_name.to_string();
    let item_pascal = meta.item_type.to_pascal_case();

    // Resolve the request parameter: first Named parameter is the JSON-serialised request payload.
    // (Streaming adapters in liter-llm pass exactly one `req: ChatCompletionRequest` argument.)
    let req_param = method.params.iter().find(|p| matches!(&p.ty, TypeRef::Named(_)));
    let (req_pascal, req_param_name) = match req_param {
        Some(p) => match &p.ty {
            TypeRef::Named(n) => (n.to_pascal_case(), p.name.to_lower_camel_case()),
            _ => (item_pascal.clone(), "req".to_string()),
        },
        None => (item_pascal.clone(), "req".to_string()),
    };
    let req_param_type = req_pascal.clone();

    let start_native = format!("{cs_type_name}{cs_method_name}Start");
    let next_native = format!("{cs_type_name}{cs_method_name}Next");
    let free_native = format!("{cs_type_name}{cs_method_name}Free");
    let req_from_json = format!("{req_pascal}FromJson");
    let req_free = format!("{req_pascal}Free");
    let item_to_json = format!("{item_pascal}ToJson");
    let item_free = format!("{item_pascal}Free");

    let mut out = String::with_capacity(2048);

    if !method.doc.is_empty() {
        out.push_str("    /// <summary>\n");
        for line in method.doc.lines() {
            out.push_str(&format!("    /// {line}\n"));
        }
        out.push_str("    /// </summary>\n");
    } else {
        out.push_str("    /// <summary>\n");
        out.push_str(&format!(
            "    /// Streaming variant of {cs_method_name}. Returns chunks as an asynchronous sequence.\n"
        ));
        out.push_str("    /// </summary>\n");
    }

    out.push_str(&format!(
        "    public async IAsyncEnumerable<{item_pascal}> {cs_method_name}(\n"
    ));
    out.push_str(&format!("        {req_param_type} {req_param_name},\n"));
    out.push_str("        [EnumeratorCancellation] CancellationToken cancellationToken = default)\n");
    out.push_str("    {\n");

    out.push_str(&format!(
        "        var {req_param_name}Json = JsonSerializer.Serialize({req_param_name}, JsonOptions);\n"
    ));
    out.push_str(&format!(
        "        var {req_param_name}Handle = NativeMethods.{req_from_json}({req_param_name}Json);\n"
    ));

    out.push_str(&format!(
        "        var streamHandle = NativeMethods.{start_native}(Handle, {req_param_name}Handle);\n"
    ));
    out.push_str("        if (streamHandle == IntPtr.Zero)\n");
    out.push_str("        {\n");
    out.push_str(&format!("            NativeMethods.{req_free}({req_param_name}Handle);\n"));
    out.push_str("            var ec = NativeMethods.LastErrorCode();\n");
    out.push_str("            var ctxPtr = NativeMethods.LastErrorContext();\n");
    out.push_str("            var msg = Marshal.PtrToStringUTF8(ctxPtr) ?? \"Unknown error\";\n");
    out.push_str(&format!("            throw new {exception_name}(ec, msg);\n"));
    out.push_str("        }\n");

    // `yield return` is incompatible with `try`/`catch`, but `try`/`finally` is fine.
    out.push_str("        try\n");
    out.push_str("        {\n");
    out.push_str("            while (true)\n");
    out.push_str("            {\n");
    out.push_str("                cancellationToken.ThrowIfCancellationRequested();\n");
    out.push_str(&format!(
        "                var chunkPtr = NativeMethods.{next_native}(streamHandle);\n"
    ));
    out.push_str("                if (chunkPtr == IntPtr.Zero)\n");
    out.push_str("                {\n");
    out.push_str("                    var ec = NativeMethods.LastErrorCode();\n");
    out.push_str("                    if (ec != 0)\n");
    out.push_str("                    {\n");
    out.push_str("                        var ctxPtr = NativeMethods.LastErrorContext();\n");
    out.push_str("                        var msg = Marshal.PtrToStringUTF8(ctxPtr) ?? \"Unknown error\";\n");
    out.push_str(&format!("                        throw new {exception_name}(ec, msg);\n"));
    out.push_str("                    }\n");
    out.push_str("                    yield break;\n");
    out.push_str("                }\n");
    out.push_str(&format!(
        "                var jsonPtr = NativeMethods.{item_to_json}(chunkPtr);\n"
    ));
    out.push_str("                var json = Marshal.PtrToStringUTF8(jsonPtr);\n");
    out.push_str("                NativeMethods.FreeString(jsonPtr);\n");
    out.push_str(&format!("                NativeMethods.{item_free}(chunkPtr);\n"));
    out.push_str(&format!(
        "                var chunk = JsonSerializer.Deserialize<{item_pascal}>(json ?? \"null\", JsonOptions)!;\n"
    ));
    out.push_str("                yield return chunk;\n");
    out.push_str("                await Task.Yield();\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        finally\n");
    out.push_str("        {\n");
    out.push_str(&format!("            NativeMethods.{free_native}(streamHandle);\n"));
    out.push_str(&format!("            NativeMethods.{req_free}({req_param_name}Handle);\n"));
    out.push_str("        }\n");
    out.push_str("    }\n");

    out
}

/// Generate a single public method on an opaque handle class.
///
/// The method delegates to `NativeMethods.{TypeName}{MethodName}(this.Handle, ...)`.
fn gen_opaque_method(
    method: &MethodDef,
    class_name: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    use crate::template_env::render;

    let mut out = String::new();

    // Collect visible params (skip any that are themselves opaque handles acting as bridges).
    let visible_params: Vec<alef_core::ir::ParamDef> = method.params.clone();

    // XML doc comment.
    if !method.doc.is_empty() {
        out.push_str("    /// <summary>\n");
        for line in method.doc.lines() {
            out.push_str(&render("doc_line_indented.jinja", minijinja::context! { line }));
        }
        out.push_str("    /// </summary>\n");
    }

    // Return type.
    let return_type_str = if method.is_async {
        if method.return_type == TypeRef::Unit {
            "async Task".to_string()
        } else {
            let return_type = csharp_type(&method.return_type);
            render("async_task_return_type.jinja", minijinja::context! { return_type })
                .trim_end_matches('\n')
                .to_string()
        }
    } else if method.return_type == TypeRef::Unit {
        "void".to_string()
    } else {
        csharp_type(&method.return_type).to_string()
    };

    let method_cs_name = to_csharp_name(&method.name);
    let is_static = method.is_static || method.receiver.is_none();
    let static_kw = if is_static { "static " } else { "" };
    out.push_str(
        render(
            "opaque_method_header.jinja",
            minijinja::context! { static_kw, return_type_str, method_cs_name },
        )
        .trim_end_matches('\n'),
    );

    // Parameters.
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

    // Serialize Named params to JSON handles.
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);

    // The native method name is {TypeName}{MethodName} (same as gen_wrapper_method).
    let cs_native_name = format!("{class_name}{method_cs_name}");

    // Result<bytes::Bytes> uses the FFI out-param convention (out_ptr/out_len/out_cap)
    // rather than the standard pointer-return marshalling. Emit a dedicated body that
    // throws via NativeMethods.LastError* directly (rather than the wrapper-class-private
    // GetLastError helper, which is not visible from this opaque-handle class).
    if super::functions::is_bytes_result_method(method) {
        let mut args_block = String::new();
        if !is_static {
            args_block.push_str("            Handle,\n");
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&format!("            {arg},\n"));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&format!("            (UIntPtr){param_name}.Length,\n"));
            }
        }
        let body = format!(
            "        var rc = NativeMethods.{cs_native_name}(\n{args_block}            out var outPtr,\n            out var outLen,\n            out var outCap\n        );\n        if (rc != 0)\n        {{\n            var ec = NativeMethods.LastErrorCode();\n            var ctxPtr = NativeMethods.LastErrorContext();\n            var msg = System.Runtime.InteropServices.Marshal.PtrToStringUTF8(ctxPtr) ?? \"Unknown error\";\n            throw new {exception_name}(ec, msg);\n        }}\n        var result = new byte[(int)outLen];\n        System.Runtime.InteropServices.Marshal.Copy(outPtr, result, 0, (int)outLen);\n        NativeMethods.FreeBytes(outPtr, outLen, outCap);\n        return result;\n",
        );
        if method.is_async {
            // Wrap synchronous P/Invoke in Task.Run to keep the async signature awaitable.
            out.push_str("        return await Task.Run(() =>\n        {\n");
            for line in body.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str("    ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            out.push_str("        });\n");
        } else {
            out.push_str(&body);
        }
        out.push_str("    }\n\n");
        return out;
    }

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

        out.push_str(&render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        ));
        if !is_static {
            out.push_str("                Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(",\n");
                out.push_str(render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(
                        render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'),
                    );
                } else {
                    out.push_str(",\n");
                    out.push_str(
                        render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'),
                    );
                }
            }
        }
        out.push_str("\n            );\n");

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "            if (nativeResult == IntPtr.Zero)\n            {\n                return null;\n            }\n",
                );
            } else {
                out.push_str(&render(
                    "null_result_throw.jinja",
                    minijinja::context! { indent => "            ", exception_name, cs_native_name },
                ));
            }
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

        out.push_str(&render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        ));
        if !is_static {
            out.push_str("            Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(",\n");
                out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                } else {
                    out.push_str(",\n");
                    out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                }
            }
        }
        out.push_str("\n        );\n");

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(
                    "        if (nativeResult == IntPtr.Zero)\n        {\n            return null;\n        }\n",
                );
            } else {
                out.push_str(&render(
                    "null_result_throw.jinja",
                    minijinja::context! { indent => "        ", exception_name, cs_native_name },
                ));
            }
        }

        emit_return_marshalling(&mut out, &method.return_type, enum_names, true_opaque_types);
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n");
    out
}

pub(super) fn gen_record_type(
    typ: &TypeDef,
    namespace: &str,
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    custom_converter_enums: &HashSet<String>,
    _lang_rename_all: &str,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    use crate::template_env::render;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    // Generate doc comment if available
    if !typ.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in typ.doc.lines() {
            out.push_str(&render("doc_line.jinja", minijinja::context! { line }));
        }
        out.push_str("/// </summary>\n");
    }

    let class_name = typ.name.to_pascal_case();
    out.push_str(&render("record_class_header.jinja", minijinja::context! { class_name }));
    out.push_str("{\n");

    for field in &typ.fields {
        // Skip unnamed tuple struct fields (e.g., _0, _1, 0, 1, etc.)
        if is_tuple_field(field) {
            continue;
        }

        // Doc comment for field
        if !field.doc.is_empty() {
            out.push_str("    /// <summary>\n");
            for line in field.doc.lines() {
                out.push_str(&render("doc_line_indented.jinja", minijinja::context! { line }));
            }
            out.push_str("    /// </summary>\n");
        }

        // Check if this field is a visitor bridge (bridge_type_alias field).
        // If so, generate special handling: IHtmlVisitor? with [JsonIgnore] instead of VisitorHandle?.
        let is_visitor_bridge = match &field.ty {
            TypeRef::Named(n) => bridge_type_aliases.contains(n),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if bridge_type_aliases.contains(n)),
            _ => false,
        };

        // Non-optional byte[] fields must be serialized as JSON int arrays, not base64.
        // Emit [JsonConverter(typeof(ByteArrayToIntArrayConverter))] for these fields.
        let needs_bytes_int_converter = !field.optional && matches!(&field.ty, TypeRef::Bytes);
        if needs_bytes_int_converter {
            out.push_str("    [JsonConverter(typeof(ByteArrayToIntArrayConverter))]\n");
        }

        // If the field's type is an enum with a custom converter, emit a property-level
        // [JsonConverter] attribute. This ensures the custom converter takes precedence
        // over the global JsonStringEnumConverter registered in JsonSerializerOptions.
        let field_base_type = match &field.ty {
            TypeRef::Named(n) => Some(n.to_pascal_case()),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.to_pascal_case()),
                _ => None,
            },
            _ => None,
        };
        if let Some(ref base) = field_base_type {
            if custom_converter_enums.contains(base) {
                out.push_str(&render("json_converter_attr.jinja", minijinja::context! { base }));
            }
        }

        // For visitor bridges, use [JsonIgnore] instead of [JsonPropertyName]
        if is_visitor_bridge {
            out.push_str("    [JsonIgnore]\n");
        } else {
            // [JsonPropertyName("json_name")]
            // FFI-based languages serialize to JSON that Rust serde deserializes.
            // Since Rust uses default snake_case, JSON property names must be snake_case.
            let json_name = field.name.clone();
            out.push_str(&render(
                "json_property_name_attr.jinja",
                minijinja::context! { json_name },
            ));
        }

        let cs_name = to_csharp_name(&field.name);

        // Check if field type is a complex enum (tagged enum with data variants).
        // These can't be simple C# enums — use JsonElement for flexible deserialization.
        let is_complex = matches!(&field.ty, TypeRef::Named(n) if complex_enums.contains(&n.to_pascal_case()));

        // Special handling for visitor bridge fields: always map to IHtmlVisitor?
        if is_visitor_bridge {
            out.push_str(&render(
                "visitor_bridge_property.jinja",
                minijinja::context! { cs_name },
            ));
            out.push('\n');
            continue;
        }

        if field.optional {
            // Optional fields: nullable type, no `required`, default = null
            let mapped = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };
            let field_type = if mapped.ends_with('?') {
                mapped
            } else {
                format!("{mapped}?")
            };
            out.push_str(&render(
                "property_with_default.jinja",
                minijinja::context! { field_type, cs_name, default_val => "null" },
            ));
        } else if typ.has_default || field.default.is_some() {
            // Field with an explicit default value or part of a type with defaults.
            // Use typed_default from IR to get Rust-compatible defaults.

            // First pass: determine what the default value will be
            let base_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };

            // Duration fields are mapped to ulong? so that 0 is distinguishable from
            // "not set". Always default to null here; Rust has its own default.
            if matches!(&field.ty, TypeRef::Duration) {
                // base_type is already "ulong?" (from csharp_type); don't add another "?"
                let nullable_type = if base_type.ends_with('?') {
                    base_type.clone()
                } else {
                    format!("{}?", base_type)
                };
                out.push_str(&render(
                    "property_with_default.jinja",
                    minijinja::context! { field_type => nullable_type, cs_name, default_val => "null" },
                ));
                out.push('\n');
                continue;
            }

            let default_val = match &field.typed_default {
                Some(DefaultValue::BoolLiteral(b)) => b.to_string(),
                Some(DefaultValue::IntLiteral(n)) => n.to_string(),
                Some(DefaultValue::FloatLiteral(f)) => {
                    let s = f.to_string();
                    let s = if s.contains('.') { s } else { format!("{s}.0") };
                    match &field.ty {
                        TypeRef::Primitive(PrimitiveType::F32) => format!("{}f", s),
                        _ => s,
                    }
                }
                Some(DefaultValue::StringLiteral(s)) => {
                    let escaped = s
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r")
                        .replace('\t', "\\t");
                    format!("\"{}\"", escaped)
                }
                Some(DefaultValue::EnumVariant(v)) => {
                    // When the C# field type is `string` (the referenced enum was excluded /
                    // collapsed to its serde JSON tag), emit the variant tag as a string literal
                    // rather than `string.VariantName` which would resolve to a missing static.
                    if base_type == "string" || base_type == "string?" {
                        format!("\"{}\"", v.to_pascal_case())
                    } else if base_type == "JsonElement" || base_type == "JsonElement?" {
                        // Complex enums mapped to JsonElement have no static variant members —
                        // default to null so the field is left unset (deserialized from JSON).
                        "null".to_string()
                    } else {
                        format!("{}.{}", base_type, v.to_pascal_case())
                    }
                }
                Some(DefaultValue::None) => "null".to_string(),
                Some(DefaultValue::Empty) | None => match &field.ty {
                    TypeRef::Vec(_) => "[]".to_string(),
                    TypeRef::Map(k, v) => format!("new Dictionary<{}, {}>()", csharp_type(k), csharp_type(v)),
                    TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "Array.Empty<byte>()".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => "false".to_string(),
                        PrimitiveType::F32 => "0.0f".to_string(),
                        PrimitiveType::F64 => "0.0".to_string(),
                        _ => "0".to_string(),
                    },
                    TypeRef::Named(name) => {
                        let pascal = name.to_pascal_case();
                        if complex_enums.contains(&pascal) {
                            // Tagged unions (complex enums) should default to null
                            "null".to_string()
                        } else if enum_names.contains(&pascal) {
                            // Plain enums with serde(default) but no explicit variant default:
                            // Default to null
                            "null".to_string()
                        } else {
                            "default!".to_string()
                        }
                    }
                    _ => "default!".to_string(),
                },
            };

            // Second pass: determine field type based on the default value
            let field_type = if (default_val == "null" && !base_type.ends_with('?')) || is_complex {
                format!("{}?", base_type)
            } else {
                base_type
            };

            out.push_str(&render(
                "property_with_default.jinja",
                minijinja::context! { field_type, cs_name, default_val },
            ));
        } else {
            // Non-optional field without explicit default.
            // Use type-appropriate zero values instead of `required` to avoid
            // JSON deserialization failures when fields are omitted via serde skip_serializing_if.
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };
            // Duration is mapped to ulong? so null is the correct "not set" default.
            if matches!(&field.ty, TypeRef::Duration) {
                out.push_str(&render(
                    "property_with_default.jinja",
                    minijinja::context! { field_type, cs_name, default_val => "null" },
                ));
            } else {
                let default_val = match &field.ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"",
                    TypeRef::Vec(_) => "[]",
                    TypeRef::Bytes => "Array.Empty<byte>()",
                    TypeRef::Primitive(PrimitiveType::Bool) => "false",
                    TypeRef::Primitive(PrimitiveType::F32) => "0.0f",
                    TypeRef::Primitive(PrimitiveType::F64) => "0.0",
                    TypeRef::Primitive(_) => "0",
                    _ => "default!",
                };
                out.push_str(&render(
                    "property_with_default.jinja",
                    minijinja::context! { field_type, cs_name, default_val },
                ));
            }
        }

        out.push('\n');
    }

    out.push_str("}\n");

    out
}

/// Generate a `ByteArrayToIntArrayConverter` class for the given namespace.
///
/// This converter serializes `byte[]` as a JSON array of integers, not base64.
/// Jackson's default serializes `byte[]` as base64, but Rust's serde expects [n, n, ...].
/// Apply with `[JsonConverter(typeof(ByteArrayToIntArrayConverter))]` on byte[] fields.
pub(crate) fn gen_byte_array_to_int_array_converter(namespace: &str) -> String {
    use crate::template_env::render;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    out.push_str("/// <summary>\n");
    out.push_str("/// Converts byte arrays to and from JSON integer arrays.\n");
    out.push_str("/// </summary>\n");
    out.push_str("/// <remarks>\n");
    out.push_str("/// System.Text.Json serializes byte[] as base64 strings by default, but Rust's serde\n");
    out.push_str("/// for Vec&lt;u8&gt; expects JSON arrays of integers [72, 101, 108, ...].\n");
    out.push_str("/// Apply this converter to byte[] fields that are serialized to FFI with\n");
    out.push_str("/// [JsonConverter(typeof(ByteArrayToIntArrayConverter))].\n");
    out.push_str("/// </remarks>\n");
    out.push_str("public sealed class ByteArrayToIntArrayConverter : JsonConverter<byte[]>\n");
    out.push_str("{\n");
    out.push_str("    /// <summary>\n");
    out.push_str("    /// Reads a JSON array of integers and converts it to a byte array.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    public override byte[]? Read(\n");
    out.push_str("        ref Utf8JsonReader reader,\n");
    out.push_str("        Type typeToConvert,\n");
    out.push_str("        JsonSerializerOptions options)\n");
    out.push_str("    {\n");
    out.push_str("        if (reader.TokenType != JsonTokenType.StartArray)\n");
    out.push_str("        {\n");
    out.push_str("            throw new JsonException(\"Expected JSON array for byte[]\");\n");
    out.push_str("        }\n\n");
    out.push_str("        var bytes = new List<byte>();\n");
    out.push_str("        while (reader.Read())\n");
    out.push_str("        {\n");
    out.push_str("            if (reader.TokenType == JsonTokenType.EndArray)\n");
    out.push_str("            {\n");
    out.push_str("                break;\n");
    out.push_str("            }\n");
    out.push_str("            if (reader.TokenType == JsonTokenType.Number)\n");
    out.push_str("            {\n");
    out.push_str("                bytes.Add((byte)reader.GetInt32());\n");
    out.push_str("            }\n");
    out.push_str("            else\n");
    out.push_str("            {\n");
    out.push_str("                throw new JsonException($\"Unexpected token type: {reader.TokenType}\");\n");
    out.push_str("            }\n");
    out.push_str("        }\n\n");
    out.push_str("        return bytes.ToArray();\n");
    out.push_str("    }\n\n");
    out.push_str("    /// <summary>\n");
    out.push_str("    /// Writes a byte array as a JSON array of integers.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    public override void Write(\n");
    out.push_str("        Utf8JsonWriter writer,\n");
    out.push_str("        byte[] value,\n");
    out.push_str("        JsonSerializerOptions options)\n");
    out.push_str("    {\n");
    out.push_str("        writer.WriteStartArray();\n");
    out.push_str("        foreach (var b in value)\n");
    out.push_str("        {\n");
    out.push_str("            writer.WriteNumberValue(b);\n");
    out.push_str("        }\n");
    out.push_str("        writer.WriteEndArray();\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}
