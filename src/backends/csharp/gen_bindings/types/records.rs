use super::super::{csharp_file_header, emit_named_param_setup, emit_named_param_teardown_indented, is_tuple_field};
use super::bridge_fields::bridge_config_for_field;
use crate::backends::csharp::type_map::{csharp_type, csharp_type_for_dto_field};
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::codegen::shared::binding_fields;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{DefaultValue, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::csharp::gen_bindings) fn gen_record_type(
    typ: &TypeDef,
    types: &[TypeDef],
    namespace: &str,
    prefix: &str,
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    custom_converter_enums: &HashSet<String>,
    _lang_rename_all: &str,
    bridge_type_aliases: &HashSet<String>,
    trait_bridges: &[TraitBridgeConfig],
    exception_class: &str,
    excluded_types: &HashSet<String>,
    tagged_union_enums: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");

    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    // Generate doc comment if available
    let typ_doc_lines = super::super::sanitize_doc_lines_for_csharp(&typ.doc);
    if !typ_doc_lines.is_empty() {
        out.push_str(&render(
            "doc_comment_block.jinja",
            minijinja::context! {
                has_doc => true,
                indent => "",
                doc_lines => typ_doc_lines,
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
        let field_doc_lines = super::super::sanitize_doc_lines_for_csharp(&field.doc);
        if !field_doc_lines.is_empty() {
            out.push_str(&render(
                "doc_comment_block.jinja",
                minijinja::context! {
                    has_doc => true,
                    indent => "    ",
                    doc_lines => field_doc_lines,
                },
            ));
        }

        // Check if this field is a visitor bridge (bridge_type_alias field).
        // If so, generate special handling: I{TraitName}? with [JsonIgnore] instead of the raw handle alias.
        let visitor_bridge = bridge_config_for_field(&field.ty, trait_bridges);
        let is_visitor_bridge = visitor_bridge.is_some()
            || match &field.ty {
                TypeRef::Named(n) => bridge_type_aliases.contains(n),
                TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), TypeRef::Named(n) if bridge_type_aliases.contains(n))
                }
                _ => false,
            };

        // byte[] fields (optional or not) must be serialized as JSON int arrays, not base64.
        // Emit [JsonConverter(typeof(ByteArrayJsonConverter))] for these fields. The converter
        // also accepts a base64 string on read, so either wire convention round-trips.
        let needs_bytes_int_converter = matches!(&field.ty, TypeRef::Bytes);
        if needs_bytes_int_converter {
            out.push_str("    [JsonConverter(typeof(ByteArrayJsonConverter))]\n");
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

        // Special handling for visitor bridge fields: always map to the configured visitor interface.
        if is_visitor_bridge {
            let interface_name = visitor_bridge
                .map(|bridge| format!("I{}", csharp_type_name(&bridge.trait_name)))
                .unwrap_or_else(|| "IVisitor".to_string());
            out.push_str(&render(
                "visitor_bridge_property.jinja",
                minijinja::context! { cs_name, interface_name },
            ));
            out.push('\n');
            continue;
        }

        if field.optional {
            // Optional fields: nullable type, no `required`, default = null
            let mapped = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type_for_dto_field(&field.ty).to_string()
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
                csharp_type_for_dto_field(&field.ty).to_string()
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

            // For non-optional primitive fields in option-config types (typ.has_default),
            // emit as nullable with null default so WhenWritingNull strips unset fields
            // and Rust applies its own defaults.
            if typ.has_default
                && field.typed_default.is_none()
                && field.default.is_none()
                && !field.optional
                && matches!(
                    &field.ty,
                    TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char | TypeRef::Path
                )
            {
                let nullable_type = if base_type.ends_with('?') {
                    base_type
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
                    // Sanitized Vec fields (e.g., tuples converted to Vec) should default to null
                    // so that Rust applies the Serde default. This prevents issues where a tuple
                    // like (1, 3) was converted to Vec<Usize> and needs the correct default on the Rust side.
                    TypeRef::Vec(_) if field.sanitized => "null".to_string(),
                    TypeRef::Vec(_) => "[]".to_string(),
                    TypeRef::Map(k, v) => {
                        format!("new Dictionary<{}, {}>()", csharp_type(k), csharp_type_for_dto_field(v))
                    }
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
                csharp_type_for_dto_field(&field.ty).to_string()
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
    out.push_str(&render(
        "record_from_json_method.jinja",
        minijinja::context! { class_name, exception_class },
    ));
    // JsonOptions is used for deserializing FFI responses (sparse JSON with defaults omitted).
    // It uses `WhenWritingDefault` to skip zero / false / null on the read side, but this
    // only affects symmetric serialization (when we round-trip objects back to FFI).
    // For sending config objects to FFI, we must use JsonSerializationOptions (no skipping)
    // so that explicitly-set false/0/null values are preserved across the FFI boundary.
    out.push_str(&render("record_json_options.jinja", minijinja::context! {}));

    // Emit record-level methods (static factories and instance withers).
    // These supersede the IntPtr-leaking counterparts on the static wrapper class:
    //   - Static method (no receiver)  → `public static ClassName Method(params)`
    //   - Instance method (has receiver) → `public ClassName Method(params)` serialising `this`
    emit_record_methods(
        &mut out,
        typ,
        types,
        &class_name,
        prefix,
        exception_class,
        true_opaque_types,
        enum_names,
    );

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
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_record_methods(
    out: &mut String,
    typ: &TypeDef,
    types: &[TypeDef],
    class_name: &str,
    _prefix: &str,
    exception_class: &str,
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) {
    use crate::backends::csharp::template_env::render;

    let native_type_prefix = class_name;

    for method in &typ.methods {
        if !matches!(&method.return_type, TypeRef::Named(name) if name == &typ.name) {
            continue;
        }

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
                    if t.ends_with('?') {
                        t.to_string()
                    } else {
                        format!("{t}?")
                    }
                } else {
                    csharp_type(&p.ty).to_string()
                };
                format!("{ptype} {pname}")
            })
            .collect();

        // Doc comment
        let sanitized_method_doc = super::super::sanitize_rust_syntax_for_csharp(&method.doc);
        if !sanitized_method_doc.trim().is_empty() {
            let first_line = sanitized_method_doc.lines().next().unwrap_or("").replace('"', "\\\"");
            out.push_str(&render("record_method_doc.jinja", minijinja::context! { first_line }));
        } else {
            out.push('\n');
        }

        let params_sig = params_sig.join(", ");
        out.push_str(&render(
            "record_method_signature.jinja",
            minijinja::context! {
                is_static => !has_receiver,
                class_name,
                method_cs_name,
                params_sig,
            },
        ));

        if method.error_type.is_some() {
            // Methods that may fail: wrap in try/catch and surface as exception.
            if has_receiver {
                // Obtain handle by serialising this instance.
                out.push_str(&render(
                    "record_self_handle_checked.jinja",
                    minijinja::context! { native_type_prefix, exception_class, class_name },
                ));
                out.push_str("        try\n        {\n");
                // Setup Named params inside try block
                emit_named_param_setup(
                    out,
                    &method.params,
                    "            ",
                    true_opaque_types,
                    exception_class,
                    types,
                    enum_names,
                );
                // Build call args using native_call_arg helper for proper marshalling
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().map(|p| {
                    super::super::native_call_arg(&p.ty, &p.name.to_lower_camel_case(), p.optional, true_opaque_types)
                }));
                let args_str = call_args.join(", ");
                out.push_str(&render(
                    "record_native_result_checked.jinja",
                    minijinja::context! {
                        indent => "            ",
                        native_method_name,
                        args_str,
                        exception_class,
                        method_cs_name,
                    },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    minijinja::context! { indent => "            ", native_type_prefix, class_name },
                ));
                out.push_str("        }\n        finally\n        {\n");
                emit_named_param_teardown_indented(out, &method.params, "            ", true_opaque_types, enum_names);
                out.push_str(&render(
                    "record_self_handle_free.jinja",
                    minijinja::context! { native_type_prefix },
                ));
                out.push_str("        }\n");
            } else {
                // Check if any params need handle setup
                let needs_handle_params = method.params.iter().any(|p| {
                    matches!(
                        &p.ty,
                        TypeRef::Named(n) if !true_opaque_types.contains(n)
                    ) || matches!(&p.ty, TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes)
                });

                if needs_handle_params {
                    emit_named_param_setup(
                        out,
                        &method.params,
                        "        ",
                        true_opaque_types,
                        exception_class,
                        types,
                        enum_names,
                    );
                    out.push_str("        try\n        {\n");
                }

                // Build call args using native_call_arg helper for proper marshalling
                let call_args: Vec<String> = method
                    .params
                    .iter()
                    .map(|p| {
                        super::super::native_call_arg(
                            &p.ty,
                            &p.name.to_lower_camel_case(),
                            p.optional,
                            true_opaque_types,
                        )
                    })
                    .collect();
                let args_str = call_args.join(", ");
                let indent = if needs_handle_params {
                    "            "
                } else {
                    "        "
                };
                out.push_str(&render(
                    "record_native_result_checked.jinja",
                    minijinja::context! {
                        indent,
                        native_method_name,
                        args_str,
                        exception_class,
                        method_cs_name,
                    },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    minijinja::context! { indent, native_type_prefix, class_name },
                ));

                if needs_handle_params {
                    out.push_str("        }\n        finally\n        {\n");
                    emit_named_param_teardown_indented(
                        out,
                        &method.params,
                        "            ",
                        true_opaque_types,
                        enum_names,
                    );
                    out.push_str("        }\n");
                }
            }
        } else {
            // Infallible methods (no error_type).
            if has_receiver {
                out.push_str(&render(
                    "record_self_handle.jinja",
                    minijinja::context! { native_type_prefix },
                ));
                out.push_str("        try\n        {\n");
                // Setup Named params inside try block
                emit_named_param_setup(
                    out,
                    &method.params,
                    "            ",
                    true_opaque_types,
                    exception_class,
                    types,
                    enum_names,
                );
                // Build call args using native_call_arg helper for proper marshalling
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().map(|p| {
                    super::super::native_call_arg(&p.ty, &p.name.to_lower_camel_case(), p.optional, true_opaque_types)
                }));
                let args_str = call_args.join(", ");
                out.push_str(&render(
                    "record_native_result.jinja",
                    minijinja::context! { indent => "            ", native_method_name, args_str },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    minijinja::context! { indent => "            ", native_type_prefix, class_name },
                ));
                out.push_str("        }\n        finally\n        {\n");
                emit_named_param_teardown_indented(out, &method.params, "            ", true_opaque_types, enum_names);
                out.push_str(&render(
                    "record_self_handle_free.jinja",
                    minijinja::context! { native_type_prefix },
                ));
                out.push_str("        }\n");
            } else {
                // Check if any params need handle setup
                let needs_handle_params = method.params.iter().any(|p| {
                    matches!(
                        &p.ty,
                        TypeRef::Named(n) if !true_opaque_types.contains(n)
                    ) || matches!(&p.ty, TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes)
                });

                if needs_handle_params {
                    emit_named_param_setup(
                        out,
                        &method.params,
                        "        ",
                        true_opaque_types,
                        exception_class,
                        types,
                        enum_names,
                    );
                    out.push_str("        try\n        {\n");
                }

                // Build call args using native_call_arg helper for proper marshalling
                let call_args: Vec<String> = method
                    .params
                    .iter()
                    .map(|p| {
                        super::super::native_call_arg(
                            &p.ty,
                            &p.name.to_lower_camel_case(),
                            p.optional,
                            true_opaque_types,
                        )
                    })
                    .collect();
                let args_str = call_args.join(", ");
                let indent = if needs_handle_params {
                    "            "
                } else {
                    "        "
                };
                out.push_str(&render(
                    "record_native_result.jinja",
                    minijinja::context! { indent, native_method_name, args_str },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    minijinja::context! { indent, native_type_prefix, class_name },
                ));

                if needs_handle_params {
                    out.push_str("        }\n        finally\n        {\n");
                    emit_named_param_teardown_indented(
                        out,
                        &method.params,
                        "            ",
                        true_opaque_types,
                        enum_names,
                    );
                    out.push_str("        }\n");
                }
            }
        }

        out.push_str("    }\n");
    }
}
