//! C# opaque handle and record type code generation.

use super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::{
    StreamingMethodMeta, csharp_file_header, emit_named_param_setup, emit_named_param_teardown,
    emit_named_param_teardown_indented, is_tuple_field, returns_ptr,
};
use crate::type_map::csharp_type;
use alef_codegen::naming::{csharp_type_name, to_csharp_name};
use alef_codegen::shared::binding_fields;
use alef_core::config::workspace::ClientConstructorConfig;
use alef_core::ir::{DefaultValue, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_handle(
    typ: &TypeDef,
    namespace: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    all_opaque_type_names: &HashSet<String>,
    client_constructor: Option<&ClientConstructorConfig>,
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

    let class_name = csharp_type_name(&typ.name);
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
                out.push_str(&gen_opaque_streaming_method(method, &class_name, exception_name, meta));
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

    // Client constructor factory method.
    if let Some(ctor) = client_constructor {
        out.push('\n');
        out.push_str(&gen_opaque_factory_method(&class_name, exception_name, ctor));
    }

    out.push_str("}\n");

    out
}

/// Map a Rust FFI type string to the C# type used in the public factory method signature.
fn ffi_ty_to_csharp_public(rust_ty: &str) -> &'static str {
    let normalized = rust_ty.trim();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return "string";
    }
    if matches!(normalized, "bool") {
        return "bool";
    }
    if matches!(normalized, "u8" | "uint8_t") {
        return "byte";
    }
    if matches!(normalized, "u16" | "uint16_t") {
        return "ushort";
    }
    if matches!(normalized, "u32" | "uint32_t") {
        return "uint";
    }
    if matches!(normalized, "u64" | "uint64_t" | "usize") {
        return "ulong";
    }
    if matches!(normalized, "i8" | "int8_t") {
        return "sbyte";
    }
    if matches!(normalized, "i16" | "int16_t") {
        return "short";
    }
    if matches!(normalized, "i32" | "int32_t" | "c_int") {
        return "int";
    }
    if matches!(normalized, "i64" | "int64_t" | "isize") {
        return "long";
    }
    if matches!(normalized, "f32" | "float") {
        return "float";
    }
    if matches!(normalized, "f64" | "double") {
        return "double";
    }
    "IntPtr"
}

/// Generate the public factory method `public static TypeName Create(params...)` that
/// calls `NativeMethods.{TypeName}New(...)` and wraps the returned handle.
fn gen_opaque_factory_method(class_name: &str, exception_name: &str, ctor: &ClientConstructorConfig) -> String {
    let mut out = String::new();

    // Public param list: `string apiKey`
    let param_list: String = ctor
        .params
        .iter()
        .map(|p| {
            let cs_type = ffi_ty_to_csharp_public(&p.ty);
            let cs_name = p.name.to_lower_camel_case();
            format!("{cs_type} {cs_name}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    // Native call arg list: pass params directly (P/Invoke handles marshalling via attributes on
    // the NativeMethods declaration).
    let call_args: String = ctor
        .params
        .iter()
        .map(|p| p.name.to_lower_camel_case())
        .collect::<Vec<_>>()
        .join(", ");

    let native_method = format!("{class_name}New");

    out.push_str(&format!(
        "    /// <summary>Creates a new <see cref=\"{class_name}\"/> handle.</summary>\n"
    ));
    out.push_str(&format!(
        "    public static {class_name} Create({param_list})\n    {{\n"
    ));
    out.push_str(&format!(
        "        var handle = NativeMethods.{native_method}({call_args});\n"
    ));
    out.push_str("        if (handle == IntPtr.Zero)\n        {\n");
    out.push_str("            var ec = NativeMethods.LastErrorCode();\n");
    out.push_str("            var ctxPtr = NativeMethods.LastErrorContext();\n");
    out.push_str(
        "            var msg = System.Runtime.InteropServices.Marshal.PtrToStringUTF8(ctxPtr) ?? \"Create failed\";\n",
    );
    out.push_str(&format!("            throw new {exception_name}(ec, msg);\n"));
    out.push_str("        }\n");
    out.push_str(&format!("        return new {class_name}(handle);\n"));
    out.push_str("    }\n");

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
    use crate::template_env::render;
    use minijinja::Value;

    let cs_method_name = to_csharp_name(&method.name);
    let cs_type_name = class_name.to_string();
    let item_pascal = csharp_type_name(&meta.item_type);

    // Resolve the request parameter: first Named parameter is the JSON-serialised request payload.
    // (Streaming adapters in liter-llm pass exactly one `req: ChatCompletionRequest` argument.)
    let req_param = method.params.iter().find(|p| matches!(&p.ty, TypeRef::Named(_)));
    let (req_pascal, req_param_name) = match req_param {
        Some(p) => match &p.ty {
            TypeRef::Named(n) => (csharp_type_name(n), p.name.to_lower_camel_case()),
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

    let doc_lines: Vec<String> = method.doc.lines().map(ToString::to_string).collect();
    render(
        "opaque_streaming_method.jinja",
        Value::from_serialize(serde_json::json!({
            "has_doc": !method.doc.is_empty(),
            "doc_lines": doc_lines,
            "method_name": cs_method_name,
            "item_type": item_pascal,
            "request_type": req_param_type,
            "request_param": req_param_name,
            "request_from_json": req_from_json,
            "request_free": req_free,
            "start_native": start_native,
            "next_native": next_native,
            "free_native": free_native,
            "item_to_json": item_to_json,
            "item_free": item_free,
            "exception_name": exception_name,
        })),
    )
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
        let doc_lines: Vec<String> = method.doc.lines().map(ToString::to_string).collect();
        out.push_str(&render(
            "doc_comment_block.jinja",
            minijinja::context! {
                has_doc => true,
                indent => "    ",
                doc_lines => doc_lines,
            },
        ));
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
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types, exception_name);

    // The native method name is {TypeName}{MethodName} (same as gen_wrapper_method).
    let cs_native_name = format!("{class_name}{method_cs_name}");

    // Result<bytes::Bytes> uses the FFI out-param convention (out_ptr/out_len/out_cap)
    // rather than the standard pointer-return marshalling. Emit a dedicated body that
    // throws via NativeMethods.LastError* directly (rather than the wrapper-class-private
    // GetLastError helper, which is not visible from this opaque-handle class).
    if super::functions::is_bytes_result_method(method) {
        let mut args_block = String::new();
        let arg_indent = if method.is_async {
            "                "
        } else {
            "            "
        };
        if !is_static {
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => arg_indent, arg => "Handle" },
            ));
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => arg_indent, arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => arg_indent, param_name },
                ));
            }
        }
        out.push_str(&render(
            "opaque_bytes_result_call.jinja",
            minijinja::context! {
                is_async => method.is_async,
                native_method_name => &cs_native_name,
                args_block => &args_block,
                exception_name,
            },
        ));
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
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_async.jinja",
                            minijinja::context! { arg => format!("(nuint){param_name}.Length") },
                        )
                        .trim_end_matches('\n'),
                    );
                }
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
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_async.jinja",
                            minijinja::context! { arg => format!("(nuint){param_name}.Length") },
                        )
                        .trim_end_matches('\n'),
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
            &HashSet::new(), // Opaque handle methods rarely return other Named types
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
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_sync.jinja",
                            minijinja::context! { arg => format!("(nuint){param_name}.Length") },
                        )
                        .trim_end_matches('\n'),
                    );
                }
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
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_sync.jinja",
                            minijinja::context! { arg => format!("(nuint){param_name}.Length") },
                        )
                        .trim_end_matches('\n'),
                    );
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

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "        ",
            enum_names,
            true_opaque_types,
            &HashSet::new(),
        );
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n");
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_record_type(
    typ: &TypeDef,
    namespace: &str,
    prefix: &str,
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    custom_converter_enums: &HashSet<String>,
    _lang_rename_all: &str,
    bridge_type_aliases: &HashSet<String>,
    exception_class: &str,
    excluded_types: &HashSet<String>,
    tagged_union_enums: &HashSet<String>,
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
        let doc_lines: Vec<String> = typ.doc.lines().map(ToString::to_string).collect();
        out.push_str(&render(
            "doc_comment_block.jinja",
            minijinja::context! {
                has_doc => true,
                indent => "",
                doc_lines => doc_lines,
            },
        ));
    }

    let class_name = csharp_type_name(&typ.name);
    out.push_str(&render("record_class_header.jinja", minijinja::context! { class_name }));
    out.push_str("{\n");

    for field in binding_fields(&typ.fields) {
        // Skip unnamed tuple struct fields (e.g., _0, _1, 0, 1, etc.)
        if is_tuple_field(field) {
            continue;
        }

        // Doc comment for field
        if !field.doc.is_empty() {
            let doc_lines: Vec<String> = field.doc.lines().map(ToString::to_string).collect();
            out.push_str(&render(
                "doc_comment_block.jinja",
                minijinja::context! {
                    has_doc => true,
                    indent => "    ",
                    doc_lines => doc_lines,
                },
            ));
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
            TypeRef::Named(n) => Some(csharp_type_name(n)),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(csharp_type_name(n)),
                _ => None,
            },
            _ => None,
        };
        if let Some(ref base) = field_base_type {
            if custom_converter_enums.contains(base) {
                out.push_str(&render("json_converter_attr.jinja", minijinja::context! { base }));
            }
        }

        // `#[serde(flatten)]` on a `serde_json::Value` field: emit
        // `[JsonExtensionData] public Dictionary<string, JsonElement> Field`
        // so System.Text.Json absorbs unknown sibling fields into the dict
        // on read, and writes them flat alongside the parent's named fields
        // on write. This mirrors the serde flatten semantic used by types
        // like `ResponseTool { tool_type, #[serde(flatten)] config: Value }`
        // where wire JSON is `{"type":"function","name":"f","description":"d"}`.
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);
        if is_flattened_json {
            let cs_name = to_csharp_name(&field.name);
            out.push_str("    [JsonExtensionData]\n");
            out.push_str(&render(
                "json_extension_data_property.jinja",
                minijinja::context! { cs_name },
            ));
            out.push('\n');
            continue;
        }

        // For visitor bridges, use [JsonIgnore] instead of [JsonPropertyName]
        if is_visitor_bridge {
            out.push_str("    [JsonIgnore]\n");
        } else {
            // [JsonPropertyName("json_name")]
            // FFI-based languages serialize to JSON that Rust serde deserializes.
            // Prefer the explicit `#[serde(rename = "...")]` value over the field name —
            // e.g. core `tool_type` with `#[serde(rename = "type")]` round-trips as
            // `"type"` on the wire, not `"tool_type"`.
            let json_name = field.serde_rename.clone().unwrap_or_else(|| field.name.clone());
            out.push_str(&render(
                "json_property_name_attr.jinja",
                minijinja::context! { json_name },
            ));
        }

        let cs_name = to_csharp_name(&field.name);

        // Check if field type is a complex enum (tagged enum with data variants) or
        // an excluded type (marked with #[alef(skip)] or #[doc(hidden)]).
        // These can't be simple C# enums — use JsonElement for flexible deserialization.
        let is_complex = matches!(&field.ty, TypeRef::Named(n) if {
            let pascal = csharp_type_name(n);
            complex_enums.contains(&pascal) || excluded_types.contains(&pascal)
        });

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
                        format!("\"{}\"", to_csharp_name(v))
                    } else if base_type == "JsonElement" || base_type == "JsonElement?" {
                        // Complex enums mapped to JsonElement have no static variant members —
                        // default to null so the field is left unset (deserialized from JSON).
                        "null".to_string()
                    } else {
                        // Tagged-union enums (serde-tagged data enums) are emitted as a C#
                        // `public abstract record Base { public sealed record Variant() : Base; }`
                        // hierarchy. `Base.Variant` therefore names a TYPE, not an instance —
                        // the property default needs `new Base.Variant()` to construct an
                        // instance, otherwise C# raises CS0119 ("X is a type, which is not
                        // valid in the given context").
                        let base_naked = base_type.trim_end_matches('?');
                        if tagged_union_enums.contains(base_naked) {
                            format!("new {}.{}()", base_naked, to_csharp_name(v))
                        } else {
                            format!("{}.{}", base_type, to_csharp_name(v))
                        }
                    }
                }
                Some(DefaultValue::None) => "null".to_string(),
                Some(DefaultValue::Empty) | None => match &field.ty {
                    TypeRef::Vec(_) => "[]".to_string(),
                    TypeRef::Map(k, v) => format!("new Dictionary<{}, {}>()", csharp_type(k), csharp_type(v)),
                    TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "[]".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => "false".to_string(),
                        PrimitiveType::F32 => "0.0f".to_string(),
                        PrimitiveType::F64 => "0.0".to_string(),
                        _ => "0".to_string(),
                    },
                    TypeRef::Named(name) => {
                        let pascal = csharp_type_name(name);
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
            // Determine if we should emit `required` modifier for non-nullable reference types.
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type(&field.ty).to_string()
            };

            // Check if this is a mandatory non-nullable reference type:
            // - String, custom classes, or complex types
            // - NOT value types (primitives, nullable types)
            // - NOT collections (collections have default [] or new())
            let should_emit_required = match &field.ty {
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
                TypeRef::Named(_) if !is_complex => true,
                TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => false,
                TypeRef::Primitive(_) => false,
                TypeRef::Duration => false,
                _ => false,
            };

            if should_emit_required {
                // Non-nullable reference type without default: use `required` modifier.
                out.push_str(&render(
                    "property_required_init.jinja",
                    minijinja::context! { field_type, cs_name },
                ));
            } else if matches!(&field.ty, TypeRef::Duration) {
                // Duration is mapped to ulong? so null is the correct "not set" default.
                out.push_str(&render(
                    "property_with_default.jinja",
                    minijinja::context! { field_type, cs_name, default_val => "null" },
                ));
            } else {
                // Value types and collections: use type-appropriate zero values.
                let default_val = match &field.ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"",
                    TypeRef::Vec(_) => "[]",
                    TypeRef::Bytes => "[]",
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

    // Emit a static `FromJson(string json)` factory that wraps any
    // System.Text.Json `JsonException` (or any other deserialization
    // failure) in `<Crate>Exception` so error fixtures can assert
    // `Assert.ThrowsAny<<Crate>Exception>(...)` over both the
    // deserialization step and the FFI call.  Without this, malformed
    // JSON surfaces as a raw `JsonException` that the test does not
    // catch.  Mirrors the Java backend's `fromJson` factory.
    out.push_str("\n    /// <summary>\n");
    out.push_str("    /// Parse a <see cref=\"");
    out.push_str(&class_name);
    out.push_str("\"/> from a JSON string.\n");
    out.push_str("    /// </summary>\n");
    out.push_str("    /// <exception cref=\"");
    out.push_str(exception_class);
    out.push_str("\">When the JSON cannot be deserialised.</exception>\n");
    out.push_str("    public static ");
    out.push_str(&class_name);
    out.push_str(" FromJson(string json)\n");
    out.push_str("    {\n");
    out.push_str("        try\n");
    out.push_str("        {\n");
    out.push_str("            return JsonSerializer.Deserialize<");
    out.push_str(&class_name);
    out.push_str(">(json, JsonOptions)\n");
    out.push_str("                ?? throw new ");
    out.push_str(exception_class);
    out.push_str("($\"Failed to parse ");
    out.push_str(&class_name);
    out.push_str(" from JSON: deserializer returned null\");\n");
    out.push_str("        }\n");
    out.push_str("        catch (");
    out.push_str(exception_class);
    out.push_str(")\n");
    out.push_str("        {\n");
    out.push_str("            throw;\n");
    out.push_str("        }\n");
    out.push_str("        catch (Exception e)\n");
    out.push_str("        {\n");
    out.push_str("            throw new ");
    out.push_str(exception_class);
    out.push_str("($\"Failed to parse ");
    out.push_str(&class_name);
    out.push_str(" from JSON: {e.Message}\", e);\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    // Match the JsonSerializerOptions used by the e2e harness's
    // `ConfigOptions` and the FFI request-serialization path so the
    // round-trip stays consistent — `JsonStringEnumConverter` with the
    // snake-case policy + `WhenWritingDefault` skipping zero / false /
    // null. Setting `PropertyNamingPolicy` explicitly here would break
    // sealed-union variant matching for records whose property names
    // already carry `[JsonPropertyName]` annotations.
    out.push_str("\n    private static readonly JsonSerializerOptions JsonOptions = new()\n");
    out.push_str("    {\n");
    out.push_str("        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingDefault,\n");
    out.push_str("        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },\n");
    out.push_str("    };\n");

    // Emit record-level methods (static factories and instance withers).
    // These supersede the IntPtr-leaking counterparts on the static wrapper class:
    //   - Static method (no receiver)  → `public static ClassName Method(params)`
    //   - Instance method (has receiver) → `public ClassName Method(params)` serialising `this`
    emit_record_methods(&mut out, typ, &class_name, prefix, exception_class);

    out.push_str("}\n");

    out
}

/// Emit record-level method wrappers for a DTO (non-opaque) type.
///
/// Static factories (no `self` receiver) are emitted as `public static {Class} Method(...)`.
/// Instance withers (`&self` receiver returning `Self`) are emitted as `public {Class} Method(...)`.
///
/// Both patterns serialise the DTO to JSON, call the FFI shim via `NativeMethods`, then
/// deserialise the returned JSON back to the record type — keeping the `IntPtr` entirely
/// internal to this method body and invisible to callers.
fn emit_record_methods(out: &mut String, typ: &TypeDef, class_name: &str, _prefix: &str, exception_class: &str) {
    let native_type_prefix = class_name;

    for method in &typ.methods {
        let method_cs_name = to_csharp_name(&method.name);
        // NativeMethods follows the pattern {TypeName}{MethodName}
        let native_method_name = format!("{native_type_prefix}{method_cs_name}");
        let has_receiver = method.receiver.is_some();

        // Build param list for signature and call args.
        let params_sig: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let pname = p.name.to_lower_camel_case();
                let ptype = if p.optional {
                    let t = csharp_type(&p.ty);
                    if t.ends_with('?') { t } else { format!("{t}?") }
                } else {
                    csharp_type(&p.ty).to_string()
                };
                format!("{ptype} {pname}")
            })
            .collect();

        // Doc comment
        if !method.doc.is_empty() {
            let first_line = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
            out.push_str(&format!("\n    /// <summary>\n    /// {first_line}\n    /// </summary>\n"));
        } else {
            out.push('\n');
        }

        if has_receiver {
            // Instance wither: `public ClassName Method(params)`
            out.push_str(&format!("    public {class_name} {method_cs_name}("));
        } else {
            // Static factory: `public static ClassName Method(params)`
            out.push_str(&format!("    public static {class_name} {method_cs_name}("));
        }

        out.push_str(&params_sig.join(", "));
        out.push_str(")\n    {\n");

        if method.error_type.is_some() {
            // Methods that may fail: wrap in try/catch and surface as exception.
            if has_receiver {
                // Obtain handle by serialising this instance.
                out.push_str(&format!(
                    "        var selfJson = JsonSerializer.Serialize(this, JsonOptions);\n\
                             var selfHandle = NativeMethods.{native_type_prefix}FromJson(selfJson);\n\
                             if (selfHandle == global::System.IntPtr.Zero) throw new {exception_class}(\"Failed to serialise {class_name}\");\n\
                             try\n        {{\n"
                ));
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().map(|p| p.name.to_lower_camel_case().to_string()));
                let args_str = call_args.join(", ");
                out.push_str(&format!(
                    "            var nativeResult = NativeMethods.{native_method_name}({args_str});\n\
                                 if (nativeResult == global::System.IntPtr.Zero) throw new {exception_class}(\"Method {method_cs_name} failed\");\n"
                ));
                out.push_str(&format!(
                    "            var jsonPtr = NativeMethods.{native_type_prefix}ToJson(nativeResult);\n\
                                 var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n\
                                 NativeMethods.FreeString(jsonPtr);\n\
                                 NativeMethods.{native_type_prefix}Free(nativeResult);\n\
                                 return JsonSerializer.Deserialize<{class_name}>(json ?? \"null\", JsonOptions)!;\n\
                         }}\n\
                         finally\n        {{\n\
                             NativeMethods.{native_type_prefix}Free(selfHandle);\n\
                         }}\n"
                ));
            } else {
                let call_args: Vec<String> = method.params.iter().map(|p| p.name.to_lower_camel_case().to_string()).collect();
                let args_str = call_args.join(", ");
                out.push_str(&format!(
                    "        var nativeResult = NativeMethods.{native_method_name}({args_str});\n\
                             if (nativeResult == global::System.IntPtr.Zero) throw new {exception_class}(\"Method {method_cs_name} failed\");\n\
                             var jsonPtr = NativeMethods.{native_type_prefix}ToJson(nativeResult);\n\
                             var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n\
                             NativeMethods.FreeString(jsonPtr);\n\
                             NativeMethods.{native_type_prefix}Free(nativeResult);\n\
                             return JsonSerializer.Deserialize<{class_name}>(json ?? \"null\", JsonOptions)!;\n"
                ));
            }
        } else {
            // Infallible methods (no error_type).
            if has_receiver {
                out.push_str(&format!(
                    "        var selfJson = JsonSerializer.Serialize(this, JsonOptions);\n\
                             var selfHandle = NativeMethods.{native_type_prefix}FromJson(selfJson);\n\
                             try\n        {{\n"
                ));
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().map(|p| p.name.to_lower_camel_case().to_string()));
                let args_str = call_args.join(", ");
                out.push_str(&format!(
                    "            var nativeResult = NativeMethods.{native_method_name}({args_str});\n\
                                 var jsonPtr = NativeMethods.{native_type_prefix}ToJson(nativeResult);\n\
                                 var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n\
                                 NativeMethods.FreeString(jsonPtr);\n\
                                 NativeMethods.{native_type_prefix}Free(nativeResult);\n\
                                 return JsonSerializer.Deserialize<{class_name}>(json ?? \"null\", JsonOptions)!;\n\
                         }}\n\
                         finally\n        {{\n\
                             NativeMethods.{native_type_prefix}Free(selfHandle);\n\
                         }}\n"
                ));
            } else {
                let call_args: Vec<String> = method.params.iter().map(|p| p.name.to_lower_camel_case().to_string()).collect();
                let args_str = call_args.join(", ");
                out.push_str(&format!(
                    "        var nativeResult = NativeMethods.{native_method_name}({args_str});\n\
                             var jsonPtr = NativeMethods.{native_type_prefix}ToJson(nativeResult);\n\
                             var json = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(jsonPtr);\n\
                             NativeMethods.FreeString(jsonPtr);\n\
                             NativeMethods.{native_type_prefix}Free(nativeResult);\n\
                             return JsonSerializer.Deserialize<{class_name}>(json ?? \"null\", JsonOptions)!;\n"
                ));
            }
        }

        out.push_str("    }\n");
    }
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
