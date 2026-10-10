use super::super::{csharp_file_header, emit_named_param_setup, emit_named_param_teardown_indented, is_tuple_field};
use super::bridge_fields::bridge_config_for_field;
use crate::backends::csharp::type_map::{csharp_type, csharp_type_for_dto_field};
use crate::codegen::naming::{csharp_type_name, field_uses_duration_map_wire, to_csharp_name};
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

    out.push_str(&render("namespace_decl.jinja", crate::alef_context! { namespace }));
    out.push('\n');

    let typ_doc_lines = super::super::sanitize_doc_lines_for_csharp(&typ.doc);
    if !typ_doc_lines.is_empty() {
        out.push_str(&render(
            "doc_comment_block.jinja",
            crate::alef_context! {
                has_doc => true,
                indent => "",
                doc_lines => typ_doc_lines,
            },
        ));
    }

    let class_name = csharp_type_name(&typ.name);
    out.push_str(&render(
        "record_class_header.jinja",
        crate::alef_context! { class_name },
    ));
    out.push_str("{\n");

    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_doc_lines = super::super::sanitize_doc_lines_for_csharp(&field.doc);
        if !field_doc_lines.is_empty() {
            out.push_str(&render(
                "doc_comment_block.jinja",
                crate::alef_context! {
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
        if let Some(ref base) = field_base_type
            && custom_converter_enums.contains(base)
        {
            out.push_str(&render("json_converter_attr.jinja", crate::alef_context! { base }));
        }

        // `#[serde(flatten)]` on a `serde_json::Value` field: emit
        // like `ResponseTool { tool_type, #[serde(flatten)] config: Value }`
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);
        if is_flattened_json {
            let cs_name = to_csharp_name(&field.name);
            out.push_str("    [JsonExtensionData]\n");
            out.push_str(&render(
                "json_extension_data_property.jinja",
                crate::alef_context! { cs_name },
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
                crate::alef_context! { json_name },
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
                crate::alef_context! { cs_name, interface_name },
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
            if field_uses_duration_map_wire(field) {
                out.push_str("    [JsonConverter(typeof(NullableDurationMillisJsonConverter))]\n");
            }
            out.push_str(&render(
                "property_with_default.jinja",
                crate::alef_context! { field_type, cs_name, default_val => "null" },
            ));
        } else if matches!(
            &field.typed_default,
            Some(DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..))
        ) {
            // `Unresolved`: alef could not read the real default out of `impl Default`.
            // `TupleVariant`/`StructVariant`: alef read the value, but this renderer has no C#
            // expression for "construct enum variant X with these field values" — neither
            // payload's arguments have a per-arg literal path here, unlike the bare
            // `EnumVariant` case below. Either way, emitting this type's zero (or a partial
            // guess at the variant's arguments) underneath a doc comment quoting the real value
            // would ship data the source never specified — silent wrong data. `required` is
            // always valid for any C# type, so it is unconditionally the honest answer here,
            // independent of `field.default`/`carries_renderable_default` and of the field's
            // type kind. ~keep
            let field_type = if is_complex {
                "JsonElement".to_string()
            } else {
                csharp_type_for_dto_field(&field.ty).to_string()
            };
            if field_uses_duration_map_wire(field) {
                out.push_str("    [JsonConverter(typeof(DurationMillisJsonConverter))]\n");
            }
            out.push_str(&render(
                "property_required_init.jinja",
                crate::alef_context! { field_type, cs_name },
            ));
        } else if field.default.is_some()
            || carries_renderable_default(field, is_complex)
            || named_struct_default_constructible(field, is_complex, types, enum_names, complex_enums, excluded_types)
        {
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
                if field_uses_duration_map_wire(field) {
                    out.push_str("    [JsonConverter(typeof(NullableDurationMillisJsonConverter))]\n");
                }
                out.push_str(&render(
                    "property_with_default.jinja",
                    crate::alef_context! { field_type => nullable_type, cs_name, default_val => "null" },
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
                Some(DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_)) => "null".to_string(),
                // A C# collection expression, which is exactly what this position already emits
                // for the empty case (`[]`), so a populated literal is valid here too. A
                // sanitized field keeps its `null` and an unrenderable element falls back to the
                // empty collection, matching the extractor's all-or-nothing rule. ~keep
                Some(DefaultValue::ListLiteral(items)) => {
                    let rendered: Option<Vec<String>> = items.iter().map(csharp_scalar_default).collect();
                    match rendered {
                        _ if field.sanitized => "null".to_string(),
                        Some(values) => format!("[{}]", values.join(", ")),
                        None => "[]".to_string(),
                    }
                }
                Some(DefaultValue::Empty) | None => match &field.ty {
                    TypeRef::Vec(_) if field.sanitized => "null".to_string(),
                    TypeRef::Named(name) => {
                        let pascal = csharp_type_name(name);
                        // An opaque type is emitted as a handle class, not a record, so it has
                        // no parameterless constructor to call. ~keep
                        if complex_enums.contains(&pascal)
                            || enum_names.contains(&pascal)
                            || true_opaque_types.contains(name)
                        {
                            "null".to_string()
                        } else {
                            nested_record_initializer(&pascal, types, enum_names, complex_enums, excluded_types)
                                .unwrap_or_else(|| "null".to_string())
                        }
                    }
                    other => csharp_type_zero_initializer(other).unwrap_or_else(|| "null".to_string()),
                },
                // The `matches!` arm above routes `Unresolved`/`TupleVariant`/`StructVariant` to
                // `required` before this match ever runs, so these arms are unreachable — kept
                // explicit rather than folded into `Empty` so a future gate change fails loudly
                // instead of silently reintroducing the fabricated-zero bug. ~keep
                Some(DefaultValue::Unresolved(_)) => {
                    unreachable!("Unresolved typed_default is handled by the branch above")
                }
                Some(DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..)) => {
                    unreachable!("TupleVariant/StructVariant typed_default is handled by the branch above")
                }
            };

            let field_type = if (default_val == "null" && !base_type.ends_with('?')) || is_complex {
                format!("{}?", base_type)
            } else {
                base_type
            };

            out.push_str(&render(
                "property_with_default.jinja",
                crate::alef_context! { field_type, cs_name, default_val },
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
                TypeRef::Duration => true,
                _ => false,
            };

            if should_emit_required {
                if field_uses_duration_map_wire(field) {
                    out.push_str("    [JsonConverter(typeof(DurationMillisJsonConverter))]\n");
                }
                out.push_str(&render(
                    "property_required_init.jinja",
                    crate::alef_context! { field_type, cs_name },
                ));
            } else {
                // `is_complex` degrades the property to the `JsonElement` struct, whose
                // `default!` is a real value rather than a null. Every other arm resolves
                // through the same table the defaulted branch uses. ~keep
                let default_val = if is_complex {
                    "default!".to_string()
                } else {
                    csharp_type_zero_initializer(&field.ty).unwrap_or_else(|| "default!".to_string())
                };
                out.push_str(&render(
                    "property_with_default.jinja",
                    crate::alef_context! { field_type, cs_name, default_val },
                ));
            }
        }

        out.push('\n');
    }

    out.push_str(&render(
        "record_from_json_method.jinja",
        crate::alef_context! { class_name, exception_class },
    ));
    out.push_str(&render("record_json_options.jinja", crate::alef_context! {}));

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

    // Properties are emitted into this same record body from `typ.fields`, and C# rejects a
    // member name used twice with `CS0102`. The property is emitted first and wins, so a
    // same-named method is dropped. Mirrors the field loop's filtering in `gen_record`. ~keep
    let property_names: HashSet<String> = binding_fields(&typ.fields)
        .filter(|field| !is_tuple_field(field))
        .map(|field| to_csharp_name(&field.name))
        .collect();

    for method in &typ.methods {
        if !matches!(&method.return_type, TypeRef::Named(name) if name == &typ.name) {
            continue;
        }

        let method_cs_name = to_csharp_name(&method.name);
        if property_names.contains(&method_cs_name) {
            continue;
        }
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
            out.push_str(&render("record_method_doc.jinja", crate::alef_context! { first_line }));
        } else {
            out.push('\n');
        }

        let params_sig = params_sig.join(", ");
        out.push_str(&render(
            "record_method_signature.jinja",
            crate::alef_context! {
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
                    crate::alef_context! { native_type_prefix, exception_class, class_name },
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
                    crate::alef_context! {
                        indent => "            ",
                        native_method_name,
                        args_str,
                        exception_class,
                        method_cs_name,
                    },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    crate::alef_context! { indent => "            ", native_type_prefix, class_name },
                ));
                out.push_str("        }\n        finally\n        {\n");
                emit_named_param_teardown_indented(out, &method.params, "            ", true_opaque_types, enum_names);
                out.push_str(&render(
                    "record_self_handle_free.jinja",
                    crate::alef_context! { native_type_prefix },
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
                    crate::alef_context! {
                        indent,
                        native_method_name,
                        args_str,
                        exception_class,
                        method_cs_name,
                    },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    crate::alef_context! { indent, native_type_prefix, class_name },
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
                    crate::alef_context! { native_type_prefix },
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
                    crate::alef_context! { indent => "            ", native_method_name, args_str },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    crate::alef_context! { indent => "            ", native_type_prefix, class_name },
                ));
                out.push_str("        }\n        finally\n        {\n");
                emit_named_param_teardown_indented(out, &method.params, "            ", true_opaque_types, enum_names);
                out.push_str(&render(
                    "record_self_handle_free.jinja",
                    crate::alef_context! { native_type_prefix },
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
                    crate::alef_context! { indent, native_method_name, args_str },
                ));
                out.push_str(&render(
                    "record_json_return.jinja",
                    crate::alef_context! { indent, native_type_prefix, class_name },
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

/// The C# expression for a `TypeRef`'s own zero, or `None` when that zero is not a value.
///
/// One table, two callers: the `Empty`/absent arm of the defaulted branch and the fallback of the
/// no-default branch. They used to spell this inline and independently, and they had drifted —
/// only the first knew about [`TypeRef::Map`], so a `HashMap` field with no `#[serde(default)]`
/// fell through to `default!` and put a **null into a non-nullable `Dictionary<K, V>` property**.
/// That is the object-initializer trap in its purest form: `new T { }` compiles, the C# nullable
/// analysis is silenced by the `!`, and the first read of the property throws.
///
/// `None` means "this type has no non-null zero in C#" — `Named` (constructing it is the caller's
/// decision, see [`nested_record_initializer`]), `Optional`/`Json` (whose zero *is* `null`, which
/// the caller must pair with a nullable property type), and `Duration` (handled before this point).
/// Returning `None` rather than inventing `default!` is the whole point: `default!` on a reference
/// type is a null wearing a non-nullable declaration. ~keep
pub(crate) fn csharp_type_zero_initializer(ty: &TypeRef) -> Option<String> {
    Some(match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "0.0f".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "0.0".to_string(),
        TypeRef::Primitive(_) => "0".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
        TypeRef::Vec(_) | TypeRef::Bytes => "[]".to_string(),
        TypeRef::Map(key, value) => format!(
            "new Dictionary<{}, {}>()",
            csharp_type(key),
            csharp_type_for_dto_field(value)
        ),
        _ => return None,
    })
}

/// The initializer for a non-optional field whose type is another emitted record, or `None` when
/// the emitter cannot safely construct one.
///
/// A nested record's own body already spells every Rust default as a property initializer, so
/// `new ContentConfig()` *is* Rust's `ContentConfig::default()` — the same guarantee the scalar
/// literals give, applied one level down. Emitting it is what makes `new CrawlConfig { MaxDepth =
/// 3 }` produce a whole value rather than one with a null `Content`.
///
/// Two things make it unsafe, and both yield `None` so the caller falls back to `null` and widens
/// the property to `T?` — a null the compiler can see beats a null hidden behind `default!`:
///
/// - The nested type declares a `required` member. `new T()` is then `CS9035`, a hard compile
///   error in the *consumer's* build, which is strictly worse than a nullable property.
/// - Constructing it would recurse forever. Rust cannot express a non-`Box` cycle, but `Box<T>`
///   can, and a cycle here is a `StackOverflowException` in generated code rather than a
///   generator error. Cheap to guard, so guard it.
fn nested_record_initializer(
    pascal: &str,
    types: &[TypeDef],
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    excluded_types: &HashSet<String>,
) -> Option<String> {
    let mut path = Vec::new();
    record_is_default_constructible(pascal, types, enum_names, complex_enums, excluded_types, &mut path)
        .then(|| format!("new {pascal}()"))
}

/// Whether a `Named`, non-enum field whose own `typed_default` is `Empty`/`None` resolves to a
/// value this renderer can construct: the field's declared type has its own zero-arg
/// constructor reachable with no `required` member of its own.
///
/// `carries_renderable_default` only ever answers `false` for a `Named` field (see its doc
/// comment) because it has no per-field context to check further — it does not know whether the
/// field's type is itself a fully default-constructible record. This is that missing check,
/// gating the same `Empty`/`None` cases `record_is_default_constructible` already verifies for a
/// *nested* record's own fields, so a directly-declared field gets the identical treatment
/// instead of unconditionally falling to `required`. `Unresolved`/`TupleVariant`/`StructVariant`
/// are excluded: those mean alef read something other than "this type's own zero", so this check
/// must not apply to them. ~keep
fn named_struct_default_constructible(
    field: &crate::core::ir::FieldDef,
    is_complex: bool,
    types: &[TypeDef],
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    excluded_types: &HashSet<String>,
) -> bool {
    if field.optional || is_complex || !matches!(&field.typed_default, Some(DefaultValue::Empty) | None) {
        return false;
    }
    let TypeRef::Named(name) = &field.ty else {
        return false;
    };
    let pascal = csharp_type_name(name);
    if enum_names.contains(&pascal) {
        return false;
    }
    let mut path = Vec::new();
    record_is_default_constructible(&pascal, types, enum_names, complex_enums, excluded_types, &mut path)
}

/// Depth limit for the cycle walk. A record graph deeper than this is pathological, and bailing to
/// a nullable property is the conservative answer either way. ~keep
const MAX_NESTED_RECORD_DEPTH: usize = 16;

fn record_is_default_constructible(
    pascal: &str,
    types: &[TypeDef],
    enum_names: &HashSet<String>,
    complex_enums: &HashSet<String>,
    excluded_types: &HashSet<String>,
    path: &mut Vec<String>,
) -> bool {
    if path.iter().any(|seen| seen == pascal) || path.len() >= MAX_NESTED_RECORD_DEPTH {
        return false;
    }
    let Some(typ) = types
        .iter()
        .find(|candidate| csharp_type_name(&candidate.name) == pascal)
    else {
        return false;
    };
    if typ.is_opaque || typ.is_trait || typ.binding_excluded || enum_names.contains(pascal) {
        return false;
    }
    // The file loop emits no `.cs` at all for a type whose only visible fields are tuple
    // positions, so `new T()` would name a class that does not exist. ~keep
    let has_visible_fields = binding_fields(&typ.fields).next().is_some();
    if has_visible_fields && !binding_fields(&typ.fields).any(|field| !is_tuple_field(field)) {
        return false;
    }

    path.push(pascal.to_string());
    let constructible = binding_fields(&typ.fields)
        .filter(|field| !is_tuple_field(field))
        .all(|field| {
            let is_complex = matches!(&field.ty, TypeRef::Named(n) if {
                let nested = csharp_type_name(n);
                complex_enums.contains(&nested) || excluded_types.contains(&nested)
            });
            // A field the recursive branch below is about to verify (a non-optional, non-enum
            // nested record whose own `typed_default` is `Empty`/`None`, i.e. it reached
            // `required` only through `field_becomes_required_property`'s bottom fallback) is
            // not disqualified here on that fallback alone: the fallback has no way to know the
            // nested record's own default is constructible — that is exactly what the recursive
            // call below determines. `Unresolved`/`TupleVariant`/`StructVariant` are excluded on
            // purpose: those mean alef read *something* other than "this type's own zero", so
            // recursing into the field's type would verify the wrong claim. Bailing here on
            // every such field would make the deep case unreachable rather than the shallow one
            // this comment already covers; every other field kind keeps the original
            // short-circuit. ~keep
            let is_recursable_nested_record = !field.optional
                && !is_complex
                && matches!(&field.ty, TypeRef::Named(n) if !enum_names.contains(&csharp_type_name(n)))
                && !matches!(
                    &field.typed_default,
                    Some(
                        DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..)
                    )
                );
            if field_becomes_required_property(field, is_complex) && !is_recursable_nested_record {
                return false;
            }
            match &field.ty {
                // Only a field that would itself construct a nested record can extend the cycle;
                // an optional one renders `X? = null` and terminates the walk. ~keep
                TypeRef::Named(n) if is_recursable_nested_record => record_is_default_constructible(
                    &csharp_type_name(n),
                    types,
                    enum_names,
                    complex_enums,
                    excluded_types,
                    path,
                ),
                _ => true,
            }
        });
    path.pop();
    constructible
}

/// Mirrors the `should_emit_required` decision in [`gen_record_type`]'s field loop: a field lands
/// on `public required T X { get; init; }` only when it is neither optional nor defaulted and its
/// type has no meaningful zero. Over-reporting here is safe (the caller falls back to a nullable
/// property); under-reporting would emit `new T()` against a type that cannot be constructed. ~keep
fn field_becomes_required_property(field: &crate::core::ir::FieldDef, is_complex: bool) -> bool {
    if field.optional || (field.serde_flatten && matches!(&field.ty, TypeRef::Json)) {
        return false;
    }
    // Mirrors the dedicated `Unresolved`/`TupleVariant`/`StructVariant` arm in
    // `gen_record_type`'s field loop: none of the three renders, so all three always render
    // `required`, regardless of `field.default`/type kind, so a nested record that has one is
    // never reported default-constructible. ~keep
    if matches!(
        &field.typed_default,
        Some(DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..))
    ) {
        return true;
    }
    if field.default.is_some() || carries_renderable_default(field, is_complex) {
        return false;
    }
    match &field.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Duration => true,
        TypeRef::Named(_) => !is_complex,
        _ => false,
    }
}

/// True when the field's own IR carries a default this emitter can turn into an initializer.
///
/// `field.typed_default` is the signal every other backend reads (java `types/records.rs`,
/// kotlin `object_wrapper/types.rs`, pyo3 `types.rs`, swift `dto.rs`, dart, go, php). It is the
/// only one that carries the *value*: `field.default` is set solely from `#[serde(default)]`
/// attributes, and `TypeDef::has_default` is a bare flag. Gating this branch on either of those
/// alone drops every `impl Default` literal on the floor and renders the type's zero value
/// instead, which is a live behaviour change across the FFI rather than a cosmetic one.
///
/// `Empty` means "that type's own `Default`". For a primitive, string, collection, bytes or
/// `serde_json::Value` field the branch's own fallback spells exactly that value, so the default
/// is renderable. For `Named` and `Duration` it does not: the branch has no initializer
/// expression for either and resolves both to `null`, widening the property to `T?`. That is a
/// hole, not a value — it lets a C# caller omit a key that the Rust `Deserialize` impl requires,
/// which is why a `Default`-deriving *struct* was never a licence to make its fields nullable.
/// Those keep `required`. A `Named` field the emitter already degrades to `JsonElement`
/// (`is_complex`) is exempt: that position is nullable either way.
fn carries_renderable_default(field: &crate::core::ir::FieldDef, is_complex: bool) -> bool {
    match &field.typed_default {
        Some(DefaultValue::Empty) => is_complex || !matches!(&field.ty, TypeRef::Named(_) | TypeRef::Duration),
        // Neither carries a value this renderer can spell: `Unresolved` because alef could not
        // read it, `TupleVariant`/`StructVariant` because this renderer has no C# expression for
        // "construct enum variant X with these arguments". Both callers must treat this exactly
        // like "no default" rather than opting into an initializer branch that would fabricate
        // the type's zero (or a partial guess at the variant's arguments). ~keep
        Some(DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..)) => false,
        Some(
            DefaultValue::BoolLiteral(_)
            | DefaultValue::StringLiteral(_)
            | DefaultValue::IntLiteral(_)
            | DefaultValue::FloatLiteral(_)
            | DefaultValue::EnumVariant(_)
            | DefaultValue::FunctionCall(_)
            | DefaultValue::PublicFunctionCall(_)
            | DefaultValue::ListLiteral(_)
            | DefaultValue::None,
        ) => true,
        None => false,
    }
}

/// Render one element of a collection-literal default as a C# expression.
///
/// Scalar-only: a nested list or a function-call default cannot be expressed in the position
/// this feeds, so both return `None` and the caller falls back to the empty collection. ~keep
fn csharp_scalar_default(item: &DefaultValue) -> Option<String> {
    match item {
        DefaultValue::BoolLiteral(b) => Some(b.to_string()),
        DefaultValue::IntLiteral(n) => Some(n.to_string()),
        DefaultValue::FloatLiteral(f) => {
            let s = f.to_string();
            Some(if s.contains('.') { s } else { format!("{s}.0") })
        }
        DefaultValue::StringLiteral(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            Some(format!("\"{escaped}\""))
        }
        DefaultValue::EnumVariant(v) => Some(format!("\"{}\"", to_csharp_name(v))),
        DefaultValue::ListLiteral(_)
        | DefaultValue::Empty
        | DefaultValue::Unresolved(_)
        | DefaultValue::TupleVariant(..)
        | DefaultValue::StructVariant(..)
        | DefaultValue::None
        | DefaultValue::FunctionCall(_)
        | DefaultValue::PublicFunctionCall(_) => None,
    }
}
