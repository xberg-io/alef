use crate::core::ir::EnumDef;

use super::super::types::{escape_kotlin_string, primitive_type_name};
use super::is_tuple_field_name;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type;
use crate::codegen::naming::wire_variant_value;
use crate::core::ir::TypeRef;

/// Emit a Jackson `StdDeserializer` for a sealed class whose enum uses the default
/// serde encoding (externally tagged) with a mix of unit + data variants. The wire
/// format is heterogeneous:
///   - Unit variants: bare JSON string `"VariantName"`.
///   - Data variants: single-keyed JSON object `{"VariantName": <inner>}` where
///     `<inner>` is the inner value serde would emit (a primitive/string for a
///     newtype/tuple variant, an object for a struct variant).
pub(super) fn emit_kotlin_heterogeneous_default_deserializer(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Deserializer : com.fasterxml.jackson.databind.deser.std.StdDeserializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    out.push_str("    @Suppress(\"LongMethod\", \"CyclomaticComplexMethod\", \"ReturnCount\")\n");
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.JsonNode>(parser)\n");
    out.push_str("        if (node.isTextual) {\n");
    out.push_str("            return when (node.asText()) {\n");
    for variant in &en.variants {
        if !variant.fields.is_empty() {
            continue;
        }
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("                \"");
        out.push_str(&escape_kotlin_string(&discriminator));
        out.push_str("\" -> ");
        out.push_str(name);
        out.push('.');
        out.push_str(&variant.name);
        out.push('\n');
    }
    out.push_str("                else -> throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("                    parser, \"Unknown ");
    out.push_str(name);
    out.push_str(" unit variant\", node.asText(), ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("                )\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        if (node.isObject) {\n");
    out.push_str("            val obj = node as com.fasterxml.jackson.databind.node.ObjectNode\n");
    out.push_str("            val it = obj.fields()\n");
    out.push_str("            if (it.hasNext()) {\n");
    out.push_str("                val entry = it.next()\n");
    out.push_str("                if (!it.hasNext()) {\n");
    out.push_str("                    val payload = entry.value\n");
    out.push_str("                    return when (entry.key) {\n");
    for variant in &en.variants {
        if variant.fields.is_empty() {
            continue;
        }
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("                        \"");
        out.push_str(&escape_kotlin_string(&discriminator));
        out.push_str("\" -> ");
        if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            let inner_class = super::kotlin_class_name_for_type(&variant.fields[0].ty);
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("(ctx.readTreeAsValue<");
            out.push_str(&inner_class);
            out.push_str(">(payload, ");
            out.push_str(&inner_class);
            out.push_str("::class.java))\n");
        } else {
            out.push_str("ctx.readTreeAsValue<");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(">(payload, ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("::class.java)\n");
        }
    }
    out.push_str("                        else -> throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("                            parser, \"Unknown ");
    out.push_str(name);
    out.push_str(" data variant\", entry.key, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("                        )\n");
    out.push_str("                    }\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("            parser, \"Cannot deserialize ");
    out.push_str(name);
    out.push_str(": expected string or single-field object\", null, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("        )\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdSerializer` for a sealed class whose enum uses the default
/// serde encoding (externally tagged) with a mix of unit + data variants. Mirrors
/// the shape produced by serde:
///   - Unit variants: write a bare string.
///   - Newtype/tuple variants: write `{"VariantName": <inner>}`.
///   - Struct variants: write `{"VariantName": {<struct fields>}}`.
pub(super) fn emit_kotlin_heterogeneous_default_serializer(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Serializer : com.fasterxml.jackson.databind.ser.std.StdSerializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun serialize(\n");
    out.push_str("        value: ");
    out.push_str(name);
    out.push_str(",\n");
    out.push_str("        gen: com.fasterxml.jackson.core.JsonGenerator,\n");
    out.push_str("        provider: com.fasterxml.jackson.databind.SerializerProvider,\n");
    out.push_str("    ) {\n");
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str("        val mapper = (gen.codec as? com.fasterxml.jackson.databind.ObjectMapper) ?: com.fasterxml.jackson.databind.ObjectMapper().findAndRegisterModules()\n");
    out.push_str("        when (value) {\n");
    for variant in &en.variants {
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("            is ");
        out.push_str(name);
        out.push('.');
        out.push_str(&variant.name);
        if variant.fields.is_empty() {
            out.push_str(" -> gen.writeString(\"");
            out.push_str(&escape_kotlin_string(&discriminator));
            out.push_str("\")\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            let field = &variant.fields[0];
            let field_name = kotlin_field_name_with_type(
                &field.name,
                0,
                match &field.ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::String => Some("String"),
                    TypeRef::Primitive(p) => Some(primitive_type_name(p)),
                    _ => None,
                },
                &variant.name,
                1,
            );
            out.push_str(" -> {\n");
            out.push_str("                gen.writeStartObject()\n");
            out.push_str("                gen.writeFieldName(\"");
            out.push_str(&escape_kotlin_string(&discriminator));
            out.push_str("\")\n");
            out.push_str("                mapper.writeValue(gen, value.");
            out.push_str(&field_name);
            out.push_str(")\n");
            out.push_str("                gen.writeEndObject()\n");
            out.push_str("            }\n");
        } else {
            out.push_str(" -> {\n");
            out.push_str("                gen.writeStartObject()\n");
            out.push_str("                gen.writeFieldName(\"");
            out.push_str(&escape_kotlin_string(&discriminator));
            out.push_str("\")\n");
            out.push_str("                mapper.writeValue(gen, value as ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(")\n");
            out.push_str("                gen.writeEndObject()\n");
            out.push_str("            }\n");
        }
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}
