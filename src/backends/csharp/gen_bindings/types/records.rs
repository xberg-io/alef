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
        if is_tuple_field(field) {
            continue;
        }

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

        let visitor_bridge = bridge_config_for_field(&field.ty, trait_bridges);
        let is_visitor_bridge = visitor_bridge.is_some()
            || match &field.ty {
                TypeRef::Named(n) => bridge_type_aliases.contains(n),
                TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), TypeRef::Named(n) if bridge_type_aliases.contains(n))
                }
                _ => false,
            };

        let needs_bytes_int_converter = matches!(&field.ty, TypeRef::Bytes);
        if needs_bytes_int_converter {
            out.push_str("    [JsonConverter(typeof(ByteArrayJsonConverter))]\n");
        }

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
        // like `ResponseTool { tool_type, #[serde(flatten)] config: Value }`
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

        if is_visitor_bridge {
            out.push_str("    [JsonIgnore]\n");
        } else {
            // Prefer the explicit `#[serde(rename = "...")]` value over the field name —
            // e.g. core `tool_type` with `#[serde(rename = "type")]` round-trips as
            let json_name = field.serde_rename.clone().unwrap_or_else(|| field.name.clone());
            out.push_str(&render(
                "json_property_name_attr.jinja",
                minijinja::context! { json_name },
            ));
        }

        let cs_name = to_csharp_name(&field.name);

        // an excluded type (marked with #[alef(skip)] or #[doc(hidden)]).
        let is_complex = matches!(&field.ty, TypeRef::Named(n) if {
            let pascal = csharp_type_name(n);
            complex_enums.contains(&pascal) || excluded_types.contains(&pascal)
        });

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
            let base_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type_for_dto_field(&field.ty).to_string()
            };

            if matches!(&field.ty, TypeRef::Duration) {
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
                    if base_type == "string" || base_type == "string?" {
                        format!("\"{}\"", to_csharp_name(v))
                    } else if base_type == "JsonElement" || base_type == "JsonElement?" {
                        "null".to_string()
                    } else {
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
                            "null".to_string()
                        } else if enum_names.contains(&pascal) {
                            "null".to_string()
                        } else {
                            "default!".to_string()
                        }
                    }
                    _ => "default!".to_string(),
                },
            };

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
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type_for_dto_field(&field.ty).to_string()
            };

            let should_emit_required = match &field.ty {
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
                TypeRef::Named(_) if !is_complex => true,
                TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => false,
                TypeRef::Primitive(_) => false,
                TypeRef::Duration => false,
                _ => false,
            };

            if should_emit_required {
                out.push_str(&render(
                    "property_required_init.jinja",
                    minijinja::context! { field_type, cs_name },
                ));
            } else if matches!(&field.ty, TypeRef::Duration) {
                out.push_str(&render(
                    "property_with_default.jinja",
                    minijinja::context! { field_type, cs_name, default_val => "null" },
                ));
            } else {
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

    out.push_str(&render(
        "record_from_json_method.jinja",
        minijinja::context! { class_name, exception_class },
    ));
    out.push_str(&render("record_json_options.jinja", minijinja::context! {}));

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

    if out.contains("GCHandle") && !out.contains("using System.Runtime.InteropServices;") {
        out = out.replacen(
            "using System.Text.Json.Serialization;\n",
            "using System.Text.Json.Serialization;\nusing System.Runtime.InteropServices;\n",
            1,
        );
    }

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
        let native_method_name = format!("{native_type_prefix}{method_cs_name}");
        let has_receiver = method.receiver.is_some();

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
            if has_receiver {
                out.push_str(&render(
                    "record_self_handle_checked.jinja",
                    minijinja::context! { native_type_prefix, exception_class, class_name },
                ));
                out.push_str("        try\n        {\n");
                emit_named_param_setup(
                    out,
                    &method.params,
                    "            ",
                    true_opaque_types,
                    exception_class,
                    types,
                    enum_names,
                );
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().flat_map(|p| {
                    let pname = p.name.to_lower_camel_case();
                    let mut a = vec![super::super::native_call_arg(
                        &p.ty,
                        &pname,
                        p.optional,
                        true_opaque_types,
                    )];
                    if matches!(p.ty, TypeRef::Bytes) {
                        a.push(super::super::bytes_len_arg("(UIntPtr)", &pname, p.optional));
                    }
                    a
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

                let call_args: Vec<String> = method
                    .params
                    .iter()
                    .flat_map(|p| {
                        let pname = p.name.to_lower_camel_case();
                        let mut a = vec![super::super::native_call_arg(
                            &p.ty,
                            &pname,
                            p.optional,
                            true_opaque_types,
                        )];
                        if matches!(p.ty, TypeRef::Bytes) {
                            a.push(super::super::bytes_len_arg("(UIntPtr)", &pname, p.optional));
                        }
                        a
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
            if has_receiver {
                out.push_str(&render(
                    "record_self_handle.jinja",
                    minijinja::context! { native_type_prefix },
                ));
                out.push_str("        try\n        {\n");
                emit_named_param_setup(
                    out,
                    &method.params,
                    "            ",
                    true_opaque_types,
                    exception_class,
                    types,
                    enum_names,
                );
                let mut call_args = vec!["selfHandle".to_string()];
                call_args.extend(method.params.iter().flat_map(|p| {
                    let pname = p.name.to_lower_camel_case();
                    let mut a = vec![super::super::native_call_arg(
                        &p.ty,
                        &pname,
                        p.optional,
                        true_opaque_types,
                    )];
                    if matches!(p.ty, TypeRef::Bytes) {
                        a.push(super::super::bytes_len_arg("(UIntPtr)", &pname, p.optional));
                    }
                    a
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

                let call_args: Vec<String> = method
                    .params
                    .iter()
                    .flat_map(|p| {
                        let pname = p.name.to_lower_camel_case();
                        let mut a = vec![super::super::native_call_arg(
                            &p.ty,
                            &pname,
                            p.optional,
                            true_opaque_types,
                        )];
                        if matches!(p.ty, TypeRef::Bytes) {
                            a.push(super::super::bytes_len_arg("(UIntPtr)", &pname, p.optional));
                        }
                        a
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
