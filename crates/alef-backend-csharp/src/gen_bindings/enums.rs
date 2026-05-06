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
/// a custom `JsonConverter<T>` that buffers all JSON properties before resolving
/// the discriminator. This is more robust than `[JsonPolymorphic]` which requires
/// the discriminator to be the first property in the JSON object.
fn gen_tagged_union(enum_def: &EnumDef, namespace: &str) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let enum_pascal = enum_def.name.to_pascal_case();
    let converter_name = format!("{enum_pascal}JsonConverter");
    // Namespace prefix used to fully-qualify inner types when their short name is shadowed
    // by a nested record of the same name (e.g. ContentPart.ImageUrl shadows ImageUrl).
    let ns = namespace;

    let mut out = csharp_file_header();
    out.push_str("using System;\n");
    out.push_str("using System.Collections.Generic;\n");
    out.push_str("using System.Text.Json;\n");
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

    // Use custom converter instead of [JsonPolymorphic] to handle discriminator in any position
    out.push_str(&format!("[JsonConverter(typeof({converter_name}))]\n"));
    out.push_str(&format!("public abstract record {enum_pascal}\n"));
    out.push_str("{\n");

    // Collect all variant pascal names to check for field-name-to-variant-name clashes
    let variant_names: std::collections::HashSet<String> =
        enum_def.variants.iter().map(|v| v.name.to_pascal_case()).collect();

    // Nested sealed records for each variant
    for variant in &enum_def.variants {
        let pascal = variant.name.to_pascal_case();

        if !variant.doc.is_empty() {
            out.push_str("    /// <summary>\n");
            for line in variant.doc.lines() {
                out.push_str(&format!("    /// {}\n", line));
            }
            out.push_str("    /// </summary>\n");
        }

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

    out.push_str("}\n\n");

    // Generate custom converter that buffers the JSON document before dispatching
    out.push_str(&format!(
        "/// <summary>Custom JSON converter for <see cref=\"{enum_pascal}\"/> that reads the \"{tag_field}\" discriminator from any position.</summary>\n"
    ));
    out.push_str(&format!(
        "internal sealed class {converter_name} : JsonConverter<{enum_pascal}>\n"
    ));
    out.push_str("{\n");

    // Read method
    out.push_str(&format!(
        "    public override {enum_pascal} Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)\n"
    ));
    out.push_str("    {\n");
    out.push_str("        using var doc = JsonDocument.ParseValue(ref reader);\n");
    out.push_str("        var root = doc.RootElement;\n");
    out.push_str(&format!(
        "        if (!root.TryGetProperty(\"{tag_field}\", out var tagEl))\n"
    ));
    out.push_str(&format!(
        "            throw new JsonException(\"{enum_pascal}: missing \\\"{tag_field}\\\" discriminator\");\n"
    ));
    out.push_str("        var tag = tagEl.GetString();\n");
    out.push_str("        var json = root.GetRawText();\n");
    out.push_str("        return tag switch\n");
    out.push_str("        {\n");

    for variant in &enum_def.variants {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        let pascal = variant.name.to_pascal_case();
        // Newtype/tuple variants have their inner type's fields inlined alongside the tag in JSON.
        // Deserialize the inner type from the full JSON object and wrap it in the record constructor.
        // Also treat single named-field variants whose parameter was renamed to "Value" (clash with
        // the variant name or the field's own type name) the same way.
        let is_tuple_newtype = variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]);
        let is_named_clash_newtype = variant.fields.len() == 1 && !is_tuple_field(&variant.fields[0]) && {
            let f = &variant.fields[0];
            let cs_type = csharp_type(&f.ty);
            let cs_name = to_csharp_name(f.name.trim_start_matches('_'));
            cs_name == pascal || cs_name == cs_type
        };
        let is_newtype = is_tuple_newtype || is_named_clash_newtype;
        if is_newtype {
            let inner_cs_type = csharp_type(&variant.fields[0].ty);
            // CS8910: when inner type name equals variant name, use object initializer
            // (no primary constructor exists — property-based record was emitted)
            if inner_cs_type == pascal {
                out.push_str(&format!(
                    "            \"{discriminator}\" => new {enum_pascal}.{pascal} {{ Value = JsonSerializer.Deserialize<{inner_cs_type}>(json, options)!\n"
                ));
                out.push_str(&format!(
                    "                ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}.Value\") }},\n"
                ));
            } else {
                out.push_str(&format!(
                    "            \"{discriminator}\" => new {enum_pascal}.{pascal}(\n"
                ));
                out.push_str(&format!(
                    "                JsonSerializer.Deserialize<{inner_cs_type}>(json, options)!\n"
                ));
                out.push_str(&format!(
                    "                    ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}.Value\")),\n"
                ));
            }
        } else {
            out.push_str(&format!(
                "            \"{discriminator}\" => JsonSerializer.Deserialize<{enum_pascal}.{pascal}>(json, options)!\n"
            ));
            out.push_str(&format!(
                "                ?? throw new JsonException(\"Failed to deserialize {enum_pascal}.{pascal}\"),\n"
            ));
        }
    }

    out.push_str(&format!(
        "            _ => throw new JsonException($\"Unknown {enum_pascal} discriminator: {{tag}}\")\n"
    ));
    out.push_str("        };\n");
    out.push_str("    }\n\n");

    // Write method
    out.push_str(&format!(
        "    public override void Write(Utf8JsonWriter writer, {enum_pascal} value, JsonSerializerOptions options)\n"
    ));
    out.push_str("    {\n");

    // Build options without this converter to avoid infinite recursion
    out.push_str("        // Serialize the concrete type, then inject the discriminator\n");
    out.push_str("        switch (value)\n");
    out.push_str("        {\n");

    for variant in &enum_def.variants {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        let pascal = variant.name.to_pascal_case();
        // Newtype/tuple variants: serialize the inner Value's fields inline alongside the tag.
        // Also applies to single named-field variants whose parameter was renamed to "Value" due
        // to a clash with the variant name or the field's own type name.
        let is_tuple_newtype = variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]);
        let is_named_clash_newtype = variant.fields.len() == 1 && !is_tuple_field(&variant.fields[0]) && {
            let f = &variant.fields[0];
            let cs_type = csharp_type(&f.ty);
            let cs_name = to_csharp_name(f.name.trim_start_matches('_'));
            cs_name == pascal || cs_name == cs_type
        };
        let is_newtype = is_tuple_newtype || is_named_clash_newtype;
        // dotnet format expects switch-case block braces indented one level
        // deeper than the `case` keyword (the body's indent), not aligned to
        // it — otherwise it reformats every commit and breaks alef-verify.
        out.push_str(&format!("            case {enum_pascal}.{pascal} v:\n"));
        out.push_str("                {\n");
        if is_newtype {
            out.push_str("                    var doc = JsonSerializer.SerializeToDocument(v.Value, options);\n");
        } else {
            out.push_str("                    var doc = JsonSerializer.SerializeToDocument(v, options);\n");
        }
        out.push_str("                    writer.WriteStartObject();\n");
        out.push_str(&format!(
            "                    writer.WriteString(\"{tag_field}\", \"{discriminator}\");\n"
        ));
        out.push_str("                    foreach (var prop in doc.RootElement.EnumerateObject())\n");
        out.push_str(&format!(
            "                        if (prop.Name != \"{tag_field}\") prop.WriteTo(writer);\n"
        ));
        out.push_str("                    writer.WriteEndObject();\n");
        out.push_str("                    break;\n");
        out.push_str("                }\n");
    }

    out.push_str(&format!(
        "            default: throw new JsonException($\"Unknown {enum_pascal} subtype: {{value.GetType().Name}}\");\n"
    ));
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}
