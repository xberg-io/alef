//! C# opaque handle and record type code generation.

use super::{csharp_file_header, is_tuple_field};
use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::{DefaultValue, PrimitiveType, TypeDef, TypeRef};
use heck::ToPascalCase;
use std::collections::HashSet;

pub(super) fn gen_opaque_handle(typ: &TypeDef, namespace: &str) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using Microsoft.Win32.SafeHandles;\n");
    out.push_str("using System.Runtime.InteropServices;\n\n");

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
    out.push_str(&format!("    private readonly {class_name}SafeHandle _safeHandle;\n\n"));
    out.push_str(&format!("    internal {class_name}(IntPtr handle)\n"));
    out.push_str("    {\n");
    out.push_str(&format!("        _safeHandle = new {class_name}SafeHandle(handle);\n"));
    out.push_str("    }\n\n");
    out.push_str("    internal IntPtr Handle => _safeHandle.DangerousGetHandle();\n\n");
    out.push_str("    public void Dispose() => _safeHandle.Dispose();\n");
    out.push_str("}\n");

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
