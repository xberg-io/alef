//! C# enum and tagged union code generation.

use super::{csharp_file_header, is_tuple_field};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::naming::{csharp_type_name, to_csharp_name, wire_variant_value};
use crate::core::ir::EnumDef;

pub(super) fn gen_enum(enum_def: &EnumDef, namespace: &str) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate abstract record hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_tagged_union(enum_def, namespace);
    }

    // Untagged union with data variants (e.g. EmbeddingInput = String | Vec<String>):
    // emit a transparent JsonElement-wrapper class with a paired JsonConverter.
    // System.Text.Json cannot dispatch between alternatives by name (variant
    // identifiers don't appear in the wire JSON), so we pass the JsonElement
    // through and let the Rust core (serde) resolve the variant.
    if enum_def.serde_untagged && has_data_variants {
        return gen_untagged_wrapper(enum_def, namespace);
    }

    // If any variant has an explicit serde_rename whose value differs from what
    // SnakeCaseLower would produce (e.g. "og:image" vs "og_image"), or if the
    // enum-level rename_all is something other than snake_case (e.g. "kebab-case"
    // for `#[serde(rename_all = "kebab-case")] FilePurpose { FineTune }` →
    // `"fine-tune"` on the wire), the global JsonStringEnumConverter(SnakeCaseLower)
    // in JsonOptions would either ignore [JsonPropertyName] (the non-generic
    // converter does not consult it on enum members) or apply the wrong policy.
    // For these cases we generate a custom JsonConverter<T> that explicitly maps
    // each variant name to the correct wire string.
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

    // Collect variant data with doc lines for template rendering
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

    // Generate custom converter for all enums (not just those with non-standard names).
    // Even when variant names match SnakeCaseLower policy, we emit a converter to ensure
    // [JsonPropertyName] attributes are properly used during serialization and to maintain
    // consistency with JsonSerializationOptions that specifies JsonStringEnumConverter(SnakeCaseLower).
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
    // Namespace prefix used to fully-qualify inner types when their short name is shadowed
    // by a nested record of the same name (e.g. ContentPart.ImageUrl shadows ImageUrl).
    let ns = namespace;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.IO;\n");
    out.push_str("using System.Text.Json;\n");
    out.push_str("using System.Text.Json.Serialization;\n\n");
    out.push_str(&render("namespace_decl.jinja", minijinja::context! { namespace }));
    out.push('\n');

    // Collect all variant pascal names to check for field-name-to-variant-name clashes
    let variant_names: std::collections::HashSet<String> =
        enum_def.variants.iter().map(|v| to_csharp_name(&v.name)).collect();

    // Precompute discriminator values for each variant (used for [JsonDerivedType] when not using custom converter)
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

    // Doc comment
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

    // Apply custom converter for sealed unions with flattened variant fields
    // This allows System.Text.Json to properly deserialize discriminator + flattened fields
    // The json_converter_attr.jinja template adds "JsonConverter" suffix, so pass just the base name
    out.push_str(&render(
        "json_converter_attr.jinja",
        minijinja::context! { base => enum_pascal },
    ));
    out.push_str(&render(
        "abstract_record_header.jinja",
        minijinja::context! { enum_pascal },
    ));
    out.push_str("{\n");

    // Nested sealed records for each variant (no [JsonDerivedType] here — it's on the base)
    // Skip binding-excluded variants
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

        if variant.fields.is_empty() {
            // Unit variant → sealed record with no fields
            out.push_str(&render(
                "unit_variant_record.jinja",
                minijinja::context! { pascal, enum_pascal },
            ));
            out.push('\n');
        } else {
            // CS8910: when a single-field variant has a parameter whose TYPE equals the record name
            // (e.g., record ImageUrl(ImageUrl Value)), the primary constructor conflicts with the
            // synthesized copy constructor. Use a property-based record body instead.
            // This applies to both tuple fields and named fields that get renamed to "Value".
            let is_copy_ctor_clash = variant.fields.len() == 1 && {
                let field_cs_type = csharp_type(&variant.fields[0].ty);
                field_cs_type.as_ref() == pascal
            };

            if is_copy_ctor_clash {
                let cs_type = csharp_type(&variant.fields[0].ty);
                // Fully qualify the inner type to avoid the nested record shadowing the
                // standalone type of the same name (e.g. `ContentPart.ImageUrl` would shadow
                // `SampleLlm.ImageUrl` within the `ContentPart` abstract record body).
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
                // Data variant → sealed record with fields as constructor params
                out.push_str(&render(
                    "variant_record_params_header.jinja",
                    minijinja::context! { pascal },
                ));
                for (i, field) in variant.fields.iter().enumerate() {
                    // If the field is sanitized (e.g., external type like ProcessResult
                    // mapped to String), use `object` as the fallback C# type to allow
                    // JSON deserialization of complex objects.
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
                    // Qualify collection types that would be shadowed by a same-named variant
                    // (e.g. NodeContent.List nested record shadows System.Collections.Generic.List<T>).
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
                        // Check if this field name clashes with:
                        // 1. The variant pascal name (e.g., "Slide" variant with "slide" field → "Slide" param)
                        // 2. The field type name (e.g., "ImageUrl" type with "url" field → "Url" param matching a nested record)
                        // 3. Another variant pascal name (e.g., nested "Title" record with "title" field in "Slide" variant)
                        let clashes = cs_name == pascal || cs_name == cs_type || variant_names.contains(&cs_name);
                        if clashes {
                            // Rename to Value with JSON property mapping to preserve the original field name
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

    // Add accessor properties for data variants
    for variant in enum_def.variants.iter().filter(|v| !v.binding_excluded) {
        // Only generate accessors for variants with exactly one tuple field
        if variant.fields.len() != 1 || !is_tuple_field(&variant.fields[0]) {
            continue;
        }
        let pascal = to_csharp_name(&variant.name);
        let field = &variant.fields[0];
        // If the field is sanitized (e.g., external type like ProcessResult
        // mapped to String), use `object` as the fallback C# type.
        let return_type = if field.sanitized && field.type_rust_path.is_some() {
            "object".to_string()
        } else {
            csharp_type(&field.ty).to_string()
        };
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

    // Generate custom converter for sealed unions with flattened variant fields
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
    // Include all variants in the converter's switch statement.
    // Non-binding-excluded variants deserialize normally.
    // Binding-excluded variants (feature-gated or hidden from bindings) emit
    // an error mentioning they are excluded, improving forward compatibility.
    let variants: Vec<Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let pascal = to_csharp_name(&v.name);
            let discriminator = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| wire_variant_value(&v.name, None, enum_def.serde_rename_all.as_deref()));
            let is_unit = v.fields.is_empty();
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
fn gen_untagged_wrapper(enum_def: &EnumDef, namespace: &str) -> String {
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
        })),
    )
}
