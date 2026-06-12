use crate::core::ir::{EnumDef, TypeRef};

use super::super::types::primitive_type_name;
use super::is_tuple_field_name;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type;
use crate::codegen::naming::wire_variant_value;

/// Emit a Jackson `StdSerializer` for an internally-tagged (`#[serde(tag = ...)]`)
/// sealed class.  The serializer adds the tag field back into the JSON object so
/// that round-tripping Kotlin → JSON → Rust works correctly.
///
/// Strategy:
/// - For **newtype/tuple variants** (single `_0` field holding an inner type):
///   serialize `value.field0` as a JSON object tree, then inject the tag field.
/// - For **named-field struct variants**: serialize the variant data class as a
///   tree (Jackson sees it as a plain data class), then inject the tag field.
/// - **Unit variants**: write `{"<tag>": "<discriminator>"}` directly.
pub(super) fn emit_kotlin_tagged_serializer(out: &mut String, en: &EnumDef, tag_field: &str) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Serializer : com.fasterxml.jackson.databind.ser.std.StdSerializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of branches scales with the number of
    // variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun serialize(\n");
    out.push_str("        value: ");
    out.push_str(name);
    out.push_str(",\n");
    out.push_str("        gen: com.fasterxml.jackson.core.JsonGenerator,\n");
    out.push_str("        provider: com.fasterxml.jackson.databind.SerializerProvider,\n");
    out.push_str("    ) {\n");
    // Use the codec as ObjectMapper so we can call valueToTree; fall back to a
    // fresh ObjectMapper if the codec is not one (shouldn't happen in practice).
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str("        val mapper = (gen.codec as? com.fasterxml.jackson.databind.ObjectMapper) ?: com.fasterxml.jackson.databind.ObjectMapper().findAndRegisterModules()\n");
    out.push_str("        val node: com.fasterxml.jackson.databind.node.ObjectNode = when (value) {\n");

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
        out.push_str(" -> {\n");

        if variant.fields.is_empty() {
            // Unit variant: emit just the tag.
            out.push_str("                val n = mapper.createObjectNode()\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: serialize the inner value as a tree then
            // inject the tag field so the output matches the tagged serde format.
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
            out.push_str("                @Suppress(\"UNCHECKED_CAST\")\n");
            out.push_str(
                "                val n = mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value.",
            );
            out.push_str(&field_name);
            out.push_str(") as com.fasterxml.jackson.databind.node.ObjectNode\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        } else {
            // Named-field struct variant: the data class carries the payload
            // fields directly.  Cast `value` to the concrete variant type before
            // calling valueToTree so Jackson resolves the serializer against the
            // variant class (which has @JsonSerialize reset to the default POJO
            // serializer), not against the parent sealed class (which would
            // re-trigger InputDocumentSerializer and cause infinite recursion).
            out.push_str("                @Suppress(\"UNCHECKED_CAST\")\n");
            out.push_str(
                "                val n = mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value as ",
            );
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(") as com.fasterxml.jackson.databind.node.ObjectNode\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        }

        out.push_str("            }\n");
    }

    out.push_str("        }\n");
    out.push_str("        mapper.writeTree(gen, node)\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdDeserializer` for an internally-tagged (`#[serde(tag = ...)]`)
/// sealed class.  The deserializer reads the tag field from the JSON object and
/// dispatches to the correct variant by calling `ctx.readTreeAsValue`.
pub(super) fn emit_kotlin_tagged_deserializer(out: &mut String, en: &EnumDef, tag_field: &str) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Deserializer : com.fasterxml.jackson.databind.deser.std.StdDeserializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of when-branches scales with the number
    // of variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.node.ObjectNode>(parser)\n");
    // Bug D fix: strip the tag field from the payload before passing it to
    // readTreeAsValue.  Inner types (e.g. SystemMessage, ContentPart.Text) do
    // not declare a `role`/`type` field, so Jackson rejects the extra key with
    // UnrecognizedPropertyException unless it is removed first.
    // Note: `deepCopy()` on `ObjectNode` is not generic in Kotlin's view of
    // the Jackson API (the Java signature `<T extends JsonNode> T deepCopy()`
    // is not callable with explicit type arguments in Kotlin 2.x), so we cast
    // the result explicitly rather than using `deepCopy<ObjectNode>()`.
    out.push_str("        val tag = node.get(\"");
    out.push_str(tag_field);
    out.push_str("\")?.asText()\n");
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str(
        "        val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"",
    );
    out.push_str(tag_field);
    out.push_str("\") }\n");
    out.push_str("        return when (tag) {\n");

    for variant in &en.variants {
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("            \"");
        out.push_str(&discriminator);
        out.push_str("\" -> ");

        if variant.fields.is_empty() {
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push('\n');
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: the `_0` IR field holds an inner named type
            // (e.g. `SystemMessage`).  Deserialize the tag-stripped payload as
            // that inner type and wrap it in the variant constructor.
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
            // Named-field struct variant: the variant data class fields are the
            // same as the JSON object fields (minus the tag).  `readTreeAsValue`
            // constructs the correct variant subtype directly from the stripped
            // payload — no constructor wrap needed.  Explicit Kotlin type
            // parameter avoids `Any!` inference on the Java generic return type.
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

    out.push_str("            else -> throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("                parser, \"Unknown ");
    out.push_str(name);
    out.push_str(" tag\", tag, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("            )\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}
