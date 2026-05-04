//! C# opaque handle and record type code generation.

use super::errors::{
    emit_return_marshalling, emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented,
};
use super::{
    csharp_file_header, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_tuple_field, returns_ptr,
};
use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::{DefaultValue, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase};
use std::collections::HashSet;

pub(super) fn gen_opaque_handle(
    typ: &TypeDef,
    namespace: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    all_opaque_type_names: &HashSet<String>,
) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using Microsoft.Win32.SafeHandles;\n");
    out.push_str("using System.Runtime.InteropServices;\n");

    // Emit additional using directives when this opaque type has methods that need JSON/async.
    let has_methods = typ.methods.iter().any(|m| !streaming_methods.contains(&m.name));
    let uses_list = |tr: &TypeRef| -> bool {
        matches!(tr, TypeRef::Vec(_))
            || matches!(tr, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)))
    };
    if has_methods {
        out.push_str("using System.Text.Json;\n");
        out.push_str("using System.Text.Json.Serialization;\n");
        let needs_list = typ
            .methods
            .iter()
            .any(|m| uses_list(&m.return_type) || m.params.iter().any(|p| uses_list(&p.ty)));
        if needs_list {
            out.push_str("using System.Collections.Generic;\n");
        }
        if typ
            .methods
            .iter()
            .any(|m| m.is_async && !streaming_methods.contains(&m.name))
        {
            out.push_str("using System.Threading.Tasks;\n");
        }
    }
    out.push('\n');

    out.push_str(&format!("namespace {};\n\n", namespace));

    let class_name = typ.name.to_pascal_case();
    let free_method = format!("{}Free", class_name);

    // Internal SafeHandle subclass — owns the native handle and calls Free on finalization.
    // Bugs 1+9: deterministic cleanup via SafeHandle; no-op Dispose() is eliminated.
    out.push_str(&format!("internal sealed class {class_name}SafeHandle : SafeHandle\n"));
    out.push_str("{\n");
    out.push_str(&format!(
        "    internal {class_name}SafeHandle(IntPtr handle) : base(IntPtr.Zero, true)\n"
    ));
    out.push_str("    {\n");
    out.push_str("        SetHandle(handle);\n");
    out.push_str("    }\n\n");
    out.push_str("    public override bool IsInvalid => handle == IntPtr.Zero;\n\n");
    out.push_str("    protected override bool ReleaseHandle()\n");
    out.push_str("    {\n");
    out.push_str(&format!("        NativeMethods.{free_method}(handle);\n"));
    out.push_str("        return true;\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // Public wrapper class — exposes IDisposable and delegates to SafeHandle.
    if !typ.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in typ.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }
    out.push_str(&format!("public sealed class {class_name} : IDisposable\n"));
    out.push_str("{\n");

    if has_methods {
        out.push_str("    private static readonly JsonSerializerOptions JsonOptions = new()\n");
        out.push_str("    {\n");
        out.push_str("        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },\n");
        out.push_str("        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingDefault\n");
        out.push_str("    };\n\n");
    }

    out.push_str(&format!("    private readonly {class_name}SafeHandle _safeHandle;\n\n"));
    out.push_str(&format!("    internal {class_name}(IntPtr handle)\n"));
    out.push_str("    {\n");
    out.push_str(&format!("        _safeHandle = new {class_name}SafeHandle(handle);\n"));
    out.push_str("    }\n\n");
    out.push_str("    internal IntPtr Handle => _safeHandle.DangerousGetHandle();\n\n");
    out.push_str("    public void Dispose() => _safeHandle.Dispose();\n");

    // Generate public methods for each non-streaming method on this opaque type.
    // These delegate to NativeMethods using this.Handle as the receiver.
    // Use the full set of opaque type names so that methods returning other opaque
    // types (e.g., LanguageRegistry::get_language → Language) are wrapped directly
    // as `new Language(ptr)` rather than being incorrectly JSON-serialized.
    let true_opaque_types = all_opaque_type_names;
    for method in typ.methods.iter().filter(|m| !streaming_methods.contains(&m.name)) {
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
    let mut out = String::new();

    // Collect visible params (skip any that are themselves opaque handles acting as bridges).
    let visible_params: Vec<alef_core::ir::ParamDef> = method.params.clone();

    // XML doc comment.
    if !method.doc.is_empty() {
        out.push_str("    /// <summary>\n");
        for line in method.doc.lines() {
            out.push_str(&format!("    /// {}\n", line));
        }
        out.push_str("    /// </summary>\n");
    }

    // Return type.
    let return_type_str = if method.is_async {
        if method.return_type == TypeRef::Unit {
            "async Task".to_string()
        } else {
            format!("async Task<{}>", csharp_type(&method.return_type))
        }
    } else if method.return_type == TypeRef::Unit {
        "void".to_string()
    } else {
        csharp_type(&method.return_type).to_string()
    };

    let method_cs_name = to_csharp_name(&method.name);
    let is_static = method.is_static || method.receiver.is_none();
    let static_kw = if is_static { "static " } else { "" };
    out.push_str(&format!("    public {static_kw}{return_type_str} {method_cs_name}("));

    // Parameters.
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

    // Serialize Named params to JSON handles.
    emit_named_param_setup(&mut out, &visible_params, "        ", true_opaque_types);

    // The native method name is {TypeName}{MethodName} (same as gen_wrapper_method).
    let cs_native_name = format!("{class_name}{method_cs_name}");

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

        out.push_str(&format!("NativeMethods.{cs_native_name}(\n"));
        if !is_static {
            out.push_str("                Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!(",\n                {arg}"));
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(&format!("                {arg}"));
                } else {
                    out.push_str(&format!(",\n                {arg}"));
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
                out.push_str(&format!(
                    "            if (nativeResult == IntPtr.Zero)\n            {{\n                throw new {exception_name}(0, \"{cs_native_name} failed\");\n            }}\n"
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

        out.push_str(&format!("NativeMethods.{cs_native_name}(\n"));
        if !is_static {
            out.push_str("            Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(&format!(",\n            {arg}"));
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(&format!("            {arg}"));
                } else {
                    out.push_str(&format!(",\n            {arg}"));
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
                out.push_str(&format!(
                    "        if (nativeResult == IntPtr.Zero)\n        {{\n            throw new {exception_name}(0, \"{cs_native_name} failed\");\n        }}\n"
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
) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&format!("namespace {};\n\n", namespace));

    // Generate doc comment if available
    if !typ.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in typ.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    out.push_str(&format!("public sealed class {}\n", typ.name.to_pascal_case()));
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
                out.push_str(&format!("    /// {}\n", line));
            }
            out.push_str("    /// </summary>\n");
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
                out.push_str(&format!("    [JsonConverter(typeof({base}JsonConverter))]\n"));
            }
        }

        // [JsonPropertyName("json_name")]
        // FFI-based languages serialize to JSON that Rust serde deserializes.
        // Since Rust uses default snake_case, JSON property names must be snake_case.
        let json_name = field.name.clone();
        out.push_str(&format!("    [JsonPropertyName(\"{}\")]\n", json_name));

        let cs_name = to_csharp_name(&field.name);

        // Check if field type is a complex enum (tagged enum with data variants).
        // These can't be simple C# enums — use JsonElement for flexible deserialization.
        let is_complex = matches!(&field.ty, TypeRef::Named(n) if complex_enums.contains(&n.to_pascal_case()));

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
            out.push_str(&format!("    public {} {} {{ get; set; }}", field_type, cs_name));
            out.push_str(" = null;\n");
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
                out.push_str(&format!(
                    "    public {} {} {{ get; set; }} = null;\n",
                    nullable_type, cs_name
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

            out.push_str(&format!(
                "    public {} {} {{ get; set; }} = {};\n",
                field_type, cs_name, default_val
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
                out.push_str(&format!(
                    "    public {} {} {{ get; set; }} = null;\n",
                    field_type, cs_name
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
                out.push_str(&format!(
                    "    public {} {} {{ get; set; }} = {};\n",
                    field_type, cs_name, default_val
                ));
            }
        }

        out.push('\n');
    }

    out.push_str("}\n");

    out
}
