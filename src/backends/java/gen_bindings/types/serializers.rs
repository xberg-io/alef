use crate::backends::java::type_map::java_type;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::EnumDef;

use crate::backends::java::gen_bindings::helpers::{is_tuple_field_name, java_apply_rename_all};

pub(crate) fn gen_byte_array_serializer(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.core.JsonGenerator",
        "com.fasterxml.jackson.databind.SerializerProvider",
        "com.fasterxml.jackson.databind.ser.std.StdSerializer",
    ];
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&crate::backends::java::template_env::render(
        "byte_array_serializer.jinja",
        minijinja::context! {},
    ));
    out
}

pub(super) fn gen_sealed_union_deserializer(out: &mut String, _package: &str, enum_def: &EnumDef, tag_field: &str) {
    // Generate the deserializer class inline in the same file
    // Start indentation at class level (not nested in the interface)
    out.push_str("// Custom deserializer for sealed interface with unwrapped variants\n");
    out.push_str("class ");
    out.push_str(&enum_def.name);
    out.push_str("Deserializer extends StdDeserializer<");
    out.push_str(&enum_def.name);
    out.push_str("> {\n");
    out.push_str("    ");
    out.push_str(&enum_def.name);
    out.push_str("Deserializer() {\n");
    out.push_str("        super(");
    out.push_str(&enum_def.name);
    out.push_str(".class);\n");
    out.push_str("    }\n\n");

    out.push_str("    @Override\n");
    out.push_str("    public ");
    out.push_str(&enum_def.name);
    out.push_str(" deserialize(JsonParser parser, DeserializationContext ctx)\n");
    out.push_str("            throws java.io.IOException {\n");
    out.push_str("        ObjectNode node = parser.getCodec().readTree(parser);\n");
    out.push_str("        com.fasterxml.jackson.databind.JsonNode tagNode = node.get(\"");
    out.push_str(tag_field);
    out.push_str("\");\n");
    out.push_str("        if (tagNode == null || tagNode.isNull()) {\n");
    out.push_str("            throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
    out.push_str("                parser, \"Missing discriminator field: ");
    out.push_str(tag_field);
    out.push_str("\");\n");
    out.push_str("        }\n");
    out.push_str("        String tagValue = tagNode.asText();\n");
    // Remove the discriminator field before deserialising the inner type so that
    // the target builder (e.g. TextMetadataBuilder) does not encounter an
    // unrecognised property and throw UnrecognizedPropertyException.
    out.push_str("        node.remove(\"");
    out.push_str(tag_field);
    out.push_str("\");\n\n");

    // Generate a switch/case based on the tag value
    out.push_str("        return switch (tagValue) {\n");
    for variant in &enum_def.variants {
        // Skip excluded variants from the deserializer switch arms
        if variant.binding_excluded {
            continue;
        }

        let discriminator = variant.serde_rename.clone().unwrap_or_else(|| {
            let name = &variant.name;
            // Apply the same naming convention as the Rust enum
            enum_def
                .serde_rename_all
                .as_deref()
                .map(|strategy| java_apply_rename_all(name, Some(strategy)))
                .unwrap_or_else(|| java_apply_rename_all(name, None))
        });

        out.push_str("            case \"");
        out.push_str(&discriminator);
        out.push_str("\" -> ");

        if variant.fields.is_empty() {
            // Unit variant
            out.push_str("new ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("();\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant - deserialize the inner type from the whole object
            let field = &variant.fields[0];
            let inner_type = java_type(&field.ty);
            out.push_str("new ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push('(');
            // For String inner types, convert the entire node to JSON string
            if inner_type.as_ref() == "String" {
                out.push_str("node.toString()");
            } else {
                out.push_str("ctx.readTreeAsValue(node, ");
                out.push_str(inner_type.as_ref());
                out.push_str(".class)");
            }
            out.push_str(");\n");
        } else {
            // Named field variant - deserialize using Jackson's normal deserialization
            out.push_str("ctx.readTreeAsValue(node, ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(".class);\n");
        }
    }

    // Check for excluded variants in the default case
    let excluded_variants: Vec<String> = enum_def
        .variants
        .iter()
        .filter(|v| v.binding_excluded)
        .map(|v| {
            let discriminator = v.serde_rename.clone().unwrap_or_else(|| {
                let name = &v.name;
                enum_def
                    .serde_rename_all
                    .as_deref()
                    .map(|strategy| java_apply_rename_all(name, Some(strategy)))
                    .unwrap_or_else(|| java_apply_rename_all(name, None))
            });
            format!("\"{}\"", discriminator)
        })
        .collect();

    out.push_str("            default -> {\n");
    if !excluded_variants.is_empty() {
        out.push_str("                if (");
        for (i, variant_discriminator) in excluded_variants.iter().enumerate() {
            if i > 0 {
                out.push_str(" || ");
            }
            out.push_str("tagValue.equals(");
            out.push_str(variant_discriminator);
            out.push(')');
        }
        out.push_str(") {\n");
        out.push_str("                    throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
        out.push_str("                        parser, \"");
        out.push_str(&enum_def.name);
        out.push_str(" variant '\" + tagValue + \"' is not available in this binding\");\n");
        out.push_str("                }\n");
    }
    out.push_str("                throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
    out.push_str("                    parser, \"Unknown ");
    out.push_str(&enum_def.name);
    out.push_str(" discriminator: \" + tagValue);\n");
    out.push_str("            }\n");
    out.push_str("        };\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit the companion serializer that mirrors `gen_sealed_union_deserializer`.
///
/// For an internally-tagged enum like `#[serde(tag = "role")] enum Message { User(UserMessage), ... }`,
/// the deserializer reads the `role` field, strips it, and dispatches to the matching variant.
/// The serializer must do the inverse: emit a flat object containing the tag field plus the
/// inner record's fields. Without this, Jackson's default serialization wraps the inner value
/// (e.g. `{"value": {...UserMessage...}}`) and Rust's serde rejects the missing tag.
pub(super) fn gen_sealed_union_serializer(out: &mut String, _package: &str, enum_def: &EnumDef, tag_field: &str) {
    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let discriminator = v.serde_rename.clone().unwrap_or_else(|| {
                let name = &v.name;
                enum_def
                    .serde_rename_all
                    .as_deref()
                    .map(|strategy| java_apply_rename_all(name, Some(strategy)))
                    .unwrap_or_else(|| java_apply_rename_all(name, None))
            });
            let is_unit = v.fields.is_empty();
            let is_tuple = !is_unit && v.fields.len() == 1 && is_tuple_field_name(&v.fields[0].name);
            minijinja::context! {
                name => &v.name,
                discriminator => discriminator,
                is_unit => is_unit,
                is_tuple => is_tuple,
            }
        })
        .collect();
    out.push_str(&crate::backends::java::template_env::render(
        "sealed_union_serializer.jinja",
        minijinja::context! {
            class_name => &enum_def.name,
            tag_field => tag_field,
            variants => variants,
        },
    ));
}
