//! C# enum and tagged union code generation.

use super::{csharp_file_header, is_tuple_field};
use crate::type_map::csharp_type;
use alef_codegen::naming::to_csharp_name;
use alef_core::ir::EnumDef;
use heck::ToPascalCase;
use std::fmt::Write;

/// Apply a serde `rename_all` strategy to a variant name.
pub(super) fn apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};

    match rename_all {
        Some("snake_case") => name.to_snake_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => name.to_snake_case().to_uppercase(),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        _ => name.to_lowercase(),
    }
}

pub(super) fn gen_enum(enum_def: &EnumDef, namespace: &str) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System.Text.Json.Serialization;\n\n");
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate abstract record hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_tagged_union(enum_def, namespace);
    }

    // If any variant has an explicit serde_rename whose value differs from what
    // SnakeCaseLower would produce (e.g. "og:image" vs "og_image"), the global
    // JsonStringEnumConverter(SnakeCaseLower) in KreuzcrawlLib.JsonOptions would
    // ignore [JsonPropertyName] and use the naming policy instead.
    // Also, the non-generic JsonStringEnumConverter does NOT support [JsonPropertyName]
    // on enum members at all. For these cases we generate a custom JsonConverter<T>
    // that explicitly maps each variant name.
    let needs_custom_converter = enum_def.variants.iter().any(|v| {
        if let Some(ref rename) = v.serde_rename {
            let snake = apply_rename_all(&v.name, enum_def.serde_rename_all.as_deref());
            rename != &snake
        } else {
            false
        }
    });

    let enum_pascal = enum_def.name.to_pascal_case();

    // Collect (json_name, pascal_name) pairs
    let variants: Vec<(String, String)> = enum_def
        .variants
        .iter()
        .map(|v| {
            let json_name = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_rename_all(&v.name, enum_def.serde_rename_all.as_deref()));
            let pascal_name = v.name.to_pascal_case();
            (json_name, pascal_name)
        })
        .collect();

    out.push_str("using System;\n");
    out.push_str("using System.Text.Json;\n\n");

    out.push_str(&format!("namespace {};\n\n", namespace));

    // Generate doc comment if available
    if !enum_def.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in enum_def.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    if needs_custom_converter {
        out.push_str(&format!("[JsonConverter(typeof({enum_pascal}JsonConverter))]\n"));
    }
    out.push_str(&format!("public enum {enum_pascal}\n"));
    out.push_str("{\n");

    for (json_name, pascal_name) in &variants {
        // Find doc for this variant
        if let Some(v) = enum_def
            .variants
            .iter()
            .find(|v| v.name.to_pascal_case() == *pascal_name)
        {
            if !v.doc.is_empty() {
                out.push_str("    /// <summary>\n");
                for line in v.doc.lines() {
                    out.push_str(&format!("    /// {}\n", line));
                }
                out.push_str("    /// </summary>\n");
            }
        }
        out.push_str(&format!("    [JsonPropertyName(\"{json_name}\")]\n"));
        out.push_str(&format!("    {pascal_name},\n"));
    }

    out.push_str("}\n");

    // Generate custom converter class after the enum when needed
    if needs_custom_converter {
        out.push('\n');
        out.push_str(&format!(
            "/// <summary>Custom JSON converter for <see cref=\"{enum_pascal}\"/> that respects explicit variant names.</summary>\n"
        ));
        out.push_str(&format!(
            "internal sealed class {enum_pascal}JsonConverter : JsonConverter<{enum_pascal}>\n"
        ));
        out.push_str("{\n");

        // Read
        out.push_str(&format!(
            "    public override {enum_pascal} Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)\n"
        ));
        out.push_str("    {\n");
        out.push_str("        var value = reader.GetString();\n");
        out.push_str("        return value switch\n");
        out.push_str("        {\n");
        for (json_name, pascal_name) in &variants {
            out.push_str(&format!(
                "            \"{json_name}\" => {enum_pascal}.{pascal_name},\n"
            ));
        }
        out.push_str(&format!(
            "            _ => throw new JsonException($\"Unknown {enum_pascal} value: {{value}}\")\n"
        ));
        out.push_str("        };\n");
        out.push_str("    }\n\n");

        // Write
        out.push_str(&format!(
            "    public override void Write(Utf8JsonWriter writer, {enum_pascal} value, JsonSerializerOptions options)\n"
        ));
        out.push_str("    {\n");
        out.push_str("        var str = value switch\n");
        out.push_str("        {\n");
        for (json_name, pascal_name) in &variants {
            out.push_str(&format!(
                "            {enum_pascal}.{pascal_name} => \"{json_name}\",\n"
            ));
        }
        out.push_str(&format!(
            "            _ => throw new JsonException($\"Unknown {enum_pascal} value: {{value}}\")\n"
        ));
        out.push_str("        };\n");
        out.push_str("        writer.WriteStringValue(str);\n");
        out.push_str("    }\n");
        out.push_str("}\n");
    }

    out
}

/// Generate a C# abstract record hierarchy for internally tagged enums.
///
/// Maps `#[serde(tag = "type_field", rename_all = "snake_case")]` Rust enums to
/// a C# polymorphic record hierarchy using .NET 7+ `[JsonPolymorphic]` and `[JsonDerivedType]`
/// attributes. These attributes are the idiomatic way to handle JSON polymorphism in modern C#.
fn gen_tagged_union(enum_def: &EnumDef, namespace: &str) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let enum_pascal = enum_def.name.to_pascal_case();
    // Namespace prefix used to fully-qualify inner types when their short name is shadowed
    // by a nested record of the same name (e.g. ContentPart.ImageUrl shadows ImageUrl).
    let ns = namespace;

    let mut out = csharp_file_header();
    out.push_str("using System.Text.Json.Serialization;\n\n");
    out.push_str(&format!("namespace {};\n\n", namespace));

    // Doc comment
    if !enum_def.doc.is_empty() {
        out.push_str("/// <summary>\n");
        for line in enum_def.doc.lines() {
            out.push_str(&format!("/// {}\n", line));
        }
        out.push_str("/// </summary>\n");
    }

    // Use [JsonPolymorphic] with the discriminator property name
    out.push_str(&format!(
        "[JsonPolymorphic(TypeDiscriminatorPropertyName = \"{tag_field}\")]\n"
    ));
    out.push_str(&format!("public abstract record {enum_pascal}\n"));
    out.push_str("{\n");

    // Collect all variant pascal names to check for field-name-to-variant-name clashes
    let variant_names: std::collections::HashSet<String> =
        enum_def.variants.iter().map(|v| v.name.to_pascal_case()).collect();

    // Nested sealed records for each variant with [JsonDerivedType] attributes
    for variant in &enum_def.variants {
        let pascal = variant.name.to_pascal_case();

        // Compute the discriminator value for this variant (the wire name)
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));

        if !variant.doc.is_empty() {
            out.push_str("    /// <summary>\n");
            for line in variant.doc.lines() {
                out.push_str(&format!("    /// {}\n", line));
            }
            out.push_str("    /// </summary>\n");
        }

        // Add [JsonDerivedType] attribute with the wire name
        out.push_str(&format!(
            "    [JsonDerivedType(typeof({pascal}), \"{discriminator}\")]\n"
        ));

        if variant.fields.is_empty() {
            // Unit variant → sealed record with no fields
            out.push_str(&format!("    public sealed record {pascal}() : {enum_pascal};\n\n"));
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
                // `LiterLlm.ImageUrl` within the `ContentPart` abstract record body).
                let qualified_cs_type = format!("global::{ns}.{cs_type}");
                out.push_str(&format!("    public sealed record {pascal} : {enum_pascal}\n"));
                out.push_str("    {\n");
                out.push_str(&format!(
                    "        public required {qualified_cs_type} Value {{ get; init; }}\n"
                ));
                out.push_str("    }\n\n");
            } else {
                // Data variant → sealed record with fields as constructor params
                out.push_str(&format!("    public sealed record {pascal}(\n"));
                for (i, field) in variant.fields.iter().enumerate() {
                    let cs_type = csharp_type(&field.ty);
                    let cs_type = if field.optional && !cs_type.ends_with('?') {
                        format!("{cs_type}?")
                    } else {
                        cs_type.to_string()
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
                        out.push_str(&format!("        {cs_type} Value{comma}\n"));
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
                            out.push_str(&format!(
                                "        [property: JsonPropertyName(\"{json_name}\")] {cs_type} Value{comma}\n"
                            ));
                        } else {
                            out.push_str(&format!(
                                "        [property: JsonPropertyName(\"{json_name}\")] {cs_type} {cs_name}{comma}\n"
                            ));
                        }
                    }
                }
                out.push_str(&format!("    ) : {enum_pascal};\n\n"));
            }
        }
    }

    // Add accessor properties for data variants
    for variant in &enum_def.variants {
        // Only generate accessors for variants with exactly one tuple field
        if variant.fields.len() != 1 || !is_tuple_field(&variant.fields[0]) {
            continue;
        }
        let pascal = variant.name.to_pascal_case();
        let return_type = csharp_type(&variant.fields[0].ty);
        let return_type_nullable = format!("{return_type}?");
        writeln!(
            out,
            "    /// <summary>Returns the {pascal} data if this is a {pascal} variant, otherwise null.</summary>"
        )
        .ok();
        writeln!(
            out,
            "    public {return_type_nullable} {pascal} => this is {pascal} e ? e.Value : null;"
        )
        .ok();
        writeln!(out).ok();
    }

    out.push_str("}\n");

    out
}
