//! C# enum and tagged union code generation.

use super::{csharp_file_header, is_tuple_field};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::naming::{csharp_type_name, to_csharp_name, wire_variant_value};
use crate::core::ir::EnumDef;

pub(super) fn gen_enum(enum_def: &EnumDef, namespace: &str, text_types: &[String]) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_tagged_union(enum_def, namespace);
    }

    if enum_def.serde_untagged && has_data_variants {
        let emit_text = text_types.iter().any(|t| t == &enum_def.name);
        return gen_untagged_wrapper(enum_def, namespace, emit_text);
    }

    // for `#[serde(rename_all = "kebab-case")] FilePurpose { FineTune }` →
    let rename_all_differs = matches!(
        enum_def.serde_rename_all.as_deref(),
        Some("kebab-case") | Some("SCREAMING-KEBAB-CASE") | Some("camelCase") | Some("PascalCase")
    );
    let needs_custom_converter = rename_all_differs
        || enum_def.variants.iter().any(|v| {
            if let Some(ref rename) = v.serde_rename {
                let default_wire_name = wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref());
                rename != &default_wire_name
            } else {
                false
            }
        });

    let enum_pascal = csharp_type_name(&enum_def.name);

    let variant_list: Vec<(String, String)> = enum_def
        .variants
        .iter()
        .map(|v| {
            let json_name = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref()));
            let pascal_name = to_csharp_name(&v.name);
            (json_name, pascal_name)
        })
        .collect();

    let variants: Vec<Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let json_name = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref()));
            let pascal_name = to_csharp_name(&v.name);
            let doc_lines = super::sanitize_doc_lines_for_csharp(&v.doc);
            let has_doc = !doc_lines.is_empty();
            Value::from_serialize(serde_json::json!({
                "json_name": json_name,
                "pascal_name": pascal_name,
                "doc": has_doc,
                "doc_lines": doc_lines,
            }))
        })
        .collect();

    let doc_lines = super::sanitize_doc_lines_for_csharp(&enum_def.doc);
    let has_doc = !doc_lines.is_empty();

    let mut out = render(
        "enum_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "enum_pascal": enum_pascal,
            "needs_custom_converter": needs_custom_converter,
            "doc": has_doc,
            "doc_lines": doc_lines,
            "variants": variants,
        })),
    );
    out.push('\n');

    out.push_str(&render(
        "enum_custom_converter.jinja",
        Value::from_serialize(serde_json::json!({
            "enum_pascal": enum_pascal,
            "variants": variant_list.iter().map(|(json_name, pascal_name)| {
                serde_json::json!({
                    "json_name": json_name,
                    "pascal_name": pascal_name,
                })
            }).collect::<Vec<_>>(),
        })),
    ));

    out
}

/// Generate a C# abstract record hierarchy for internally tagged enums.
///
/// Maps `#[serde(tag = "type_field", rename_all = "snake_case")]` Rust enums to
/// a C# polymorphic record hierarchy using .NET 7+ `[JsonPolymorphic]` and `[JsonDerivedType]`
/// attributes. These attributes are the idiomatic way to handle JSON polymorphism in modern C#.
fn gen_tagged_union(enum_def: &EnumDef, namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let enum_pascal = csharp_type_name(&enum_def.name);
    let ns = namespace;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.IO;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");
    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    let variant_names: std::collections::HashSet<String> =
        enum_def.variants.iter().map(|v| to_csharp_name(&v.name)).collect();

    let _discriminators: Vec<(String, String)> = enum_def
        .variants
        .iter()
        .map(|v| {
            let pascal = to_csharp_name(&v.name);
            let disc = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref()));
            (pascal, disc)
        })
        .collect();

    let enum_doc_lines = super::sanitize_doc_lines_for_csharp(&enum_def.doc);
    if !enum_doc_lines.is_empty() {
        out.push_str(&render(
            "doc_comment_block.jinja",
            minijinja::context! {
                has_doc => true,
                indent => "",
                doc_lines => enum_doc_lines,
            },
        ));
    }

    out.push_str(&render(
        "json_converter_attr.jinja",
        minijinja::context! { base => enum_pascal },
    ));
    out.push_str(&render(
        "abstract_record_header.jinja",
        minijinja::context! { enum_pascal },
    ));
    out.push_str("{\n");

    for variant in enum_def.variants.iter().filter(|v| !v.binding_excluded) {
        let pascal = to_csharp_name(&variant.name);

        let variant_doc_lines = super::sanitize_doc_lines_for_csharp(&variant.doc);
        if !variant_doc_lines.is_empty() {
            out.push_str(&render(
                "doc_comment_block.jinja",
                minijinja::context! {
                    has_doc => true,
                    indent => "    ",
                    doc_lines => variant_doc_lines,
                },
            ));
        }

        let is_unit_tuple = variant.fields.len() == 1
            && is_tuple_field(&variant.fields[0])
            && csharp_type(&variant.fields[0].ty).as_ref() == "void";
        if variant.fields.is_empty() || is_unit_tuple {
            out.push_str(&render(
                "unit_variant_record.jinja",
                minijinja::context! { pascal, enum_pascal },
            ));
            out.push('\n');
        } else {
            let is_copy_ctor_clash = variant.fields.len() == 1 && {
                let field_cs_type = csharp_type(&variant.fields[0].ty);
                field_cs_type.as_ref() == pascal
            };

            if is_copy_ctor_clash {
                let cs_type = csharp_type(&variant.fields[0].ty);
                let qualified_cs_type = format!("global::{ns}.{cs_type}");
                out.push_str(&render(
                    "variant_record_body_header.jinja",
                    minijinja::context! { pascal, enum_pascal },
                ));
                out.push_str("    {\n");
                out.push_str(&render(
                    "required_value_property.jinja",
                    minijinja::context! { qualified_cs_type },
                ));
                out.push_str("    }\n\n");
            } else {
                out.push_str(&render(
                    "variant_record_params_header.jinja",
                    minijinja::context! { pascal },
                ));
                for (i, field) in variant.fields.iter().enumerate() {
                    let cs_type = if field.sanitized && field.type_rust_path.is_some() {
                        "object".to_string()
                    } else {
                        csharp_type(&field.ty).to_string()
                    };
                    let cs_type = if field.optional && !cs_type.ends_with('?') {
                        format!("{cs_type}?")
                    } else {
                        cs_type
                    };
                    let cs_type = if variant_names.iter().any(|vn| cs_type.starts_with(&format!("{vn}<"))) {
                        cs_type
                            .replace("List<", "global::System.Collections.Generic.List<")
                            .replace("Dictionary<", "global::System.Collections.Generic.Dictionary<")
                    } else {
                        cs_type
                    };
                    let comma = if i < variant.fields.len() - 1 { "," } else { "" };
                    if is_tuple_field(field) {
                        out.push_str(&render(
                            "variant_field_tuple.jinja",
                            minijinja::context! { cs_type, comma },
                        ));
                    } else {
                        let json_name = field.name.trim_start_matches('_');
                        let cs_name = to_csharp_name(json_name);
                        let clashes = cs_name == pascal || cs_name == cs_type || variant_names.contains(&cs_name);
                        if clashes {
                            out.push_str(&render(
                                "variant_field_json_value.jinja",
                                minijinja::context! { json_name, cs_type, comma },
                            ));
                        } else {
                            out.push_str(&render(
                                "variant_field_json_named.jinja",
                                minijinja::context! { json_name, cs_type, cs_name, comma },
                            ));
                        }
                    }
                }
                out.push_str(&render(
                    "variant_record_close.jinja",
                    minijinja::context! { enum_pascal },
                ));
                out.push('\n');
            }
        }
    }

    for variant in enum_def.variants.iter().filter(|v| !v.binding_excluded) {
        if variant.fields.len() != 1 || !is_tuple_field(&variant.fields[0]) {
            continue;
        }
        let pascal = to_csharp_name(&variant.name);
        let field = &variant.fields[0];
        let return_type = if field.sanitized && field.type_rust_path.is_some() {
            "object".to_string()
        } else {
            csharp_type(&field.ty).to_string()
        };
        if return_type == "void" {
            continue;
        }
        let return_type_nullable = format!("{return_type}?");
        out.push_str(&render(
            "variant_accessor_summary.jinja",
            minijinja::context! { pascal },
        ));
        out.push_str(&render(
            "variant_accessor_property.jinja",
            minijinja::context! { pascal, return_type_nullable },
        ));
        out.push('\n');
    }

    out.push_str("}\n");

    out.push('\n');
    gen_sealed_union_converter(&mut out, namespace, enum_def, tag_field);

    out
}

/// Generate a custom JsonConverter for sealed unions with flattened (unwrapped) variant fields.
///
/// This converter handles tagged unions where the discriminator field (`format_type`, etc.)
/// is followed by variant-specific fields all at the same JSON level:
///
/// ```json
/// {
///   "format_type": "excel",
///   "sheet_count": 2,
///   "sheet_names": ["Sheet1", "Sheet2"]
/// }
/// ```
///
/// System.Text.Json's [JsonPolymorphic] + [JsonDerivedType] cannot handle this layout directly
/// because the variant's nested record expects its fields as JSON members but doesn't know to
/// ignore the discriminator. This converter manually parses the JSON, reads the discriminator,
/// removes it, and deserializes the remaining fields into the appropriate variant type.
fn gen_sealed_union_converter(out: &mut String, _namespace: &str, enum_def: &EnumDef, tag_field: &str) {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let class_name = csharp_type_name(&enum_def.name);
    let variants: Vec<Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let pascal = to_csharp_name(&v.name);
            let discriminator = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref()));
            let is_unit_tuple =
                v.fields.len() == 1 && is_tuple_field(&v.fields[0]) && csharp_type(&v.fields[0].ty).as_ref() == "void";
            let is_unit = v.fields.is_empty() || is_unit_tuple;
            let is_tuple = !is_unit && v.fields.len() == 1 && is_tuple_field(&v.fields[0]);
            let is_excluded = v.binding_excluded;
            Value::from_serialize(serde_json::json!({
                "pascal": pascal,
                "pascal_lower": pascal.to_lowercase(),
                "discriminator": discriminator,
                "is_unit": is_unit,
                "is_tuple": is_tuple,
                "is_excluded": is_excluded,
            }))
        })
        .collect();
    out.push_str(&render(
        "sealed_union_converter.jinja",
        Value::from_serialize(serde_json::json!({
            "class_name": class_name,
            "tag_field": tag_field,
            "variants": variants,
        })),
    ));
}

/// Emit a transparent JsonElement-wrapper class for `#[serde(untagged)]` enums.
///
/// Untagged unions like `EmbeddingInput = Single(String) | Multiple(Vec<String>)`
/// have no on-wire discriminator. The default System.Text.Json enum converter
/// rejects any value that doesn't match a variant name. The wrapper class holds
/// the JsonElement verbatim, with a paired JsonConverter that round-trips the
/// raw JSON. Static factories (`Of`, `FromJson`, `OfObject`) and probe accessors
/// (`AsString`, `AsList`, `AsObject`) keep ergonomic construction available.
///
/// When `emit_text` is true, a `Text()` method is appended that returns the
/// plain-text display value: a JSON string is returned verbatim; a JSON array of
/// objects with `"type":"text"` has their `"text"` fields concatenated;
/// anything else returns `""`.
fn gen_untagged_wrapper(enum_def: &EnumDef, namespace: &str, emit_text: bool) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let class_name = csharp_type_name(&enum_def.name);
    let doc_lines = super::sanitize_doc_lines_for_csharp(&enum_def.doc);
    let has_doc = !doc_lines.is_empty();

    render(
        "untagged_union_wrapper.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "doc": has_doc,
            "doc_lines": doc_lines,
            "emit_text": emit_text,
        })),
    )
}
