use crate::core::ir::{EnumDef, TypeRef};

use super::super::types::primitive_type_name;
use super::is_tuple_field_name;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type;

/// Emit a `fun text(): String` method on an untagged sealed class that extracts
/// the plain-text display value.
///
/// Semantics mirror Rust's Display trait:
/// - If the sealed class value is a newtype variant wrapping a JSON string,
///   return that string verbatim.
/// - If the value is a newtype variant wrapping a JSON array of objects with
///   `"type":"text"`, concatenate the `"text"` field of each text-type part.
/// - Otherwise, return an empty string.
///
/// This function is only emitted when `emit_text` is true (i.e., the enum name
/// is in the config's `untagged_union_text_types` list).
pub(super) fn emit_kotlin_text_accessor(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push_str("    /**\n");
    out.push_str("     * Returns the plain-text display value of this content.\n");
    out.push_str("     *\n");
    out.push_str("     * - If the value is a JSON string, it is returned verbatim.\n");
    out.push_str("     * - If the value is a JSON array, the `\"text\"` field of every\n");
    out.push_str("     *   element whose `\"type\"` equals `\"text\"` is concatenated in order;\n");
    out.push_str("     *   non-text parts (images, audio, refusals, etc.) are skipped.\n");
    out.push_str("     * - Otherwise (null, object, empty) returns an empty string.\n");
    out.push_str("     */\n");
    out.push_str("    fun text(): String = when (this) {\n");

    for variant in &en.variants {
        if variant.fields.is_empty() {
            out.push_str("        is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> \"\"\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            let field = &variant.fields[0];
            match &field.ty {
                TypeRef::String => {
                    out.push_str("        is ");
                    out.push_str(name);
                    out.push('.');
                    out.push_str(&variant.name);
                    out.push_str(" -> this.value\n");
                }
                TypeRef::Vec(elem_ty) => {
                    if let TypeRef::Named(_) | TypeRef::Json = **elem_ty {
                        out.push_str("        is ");
                        out.push_str(name);
                        out.push('.');
                        out.push_str(&variant.name);
                        out.push_str(" -> {\n");
                        out.push_str("            val sb = StringBuilder()\n");
                        out.push_str("            for (part in this.value) {\n");
                        out.push_str("                if (part is com.fasterxml.jackson.databind.JsonNode) {\n");
                        out.push_str("                    val typeNode = part.get(\"type\")\n");
                        out.push_str("                    if (typeNode?.asText() == \"text\") {\n");
                        out.push_str("                        val textNode = part.get(\"text\")\n");
                        out.push_str("                        if (textNode != null) {\n");
                        out.push_str("                            sb.append(textNode.asText())\n");
                        out.push_str("                        }\n");
                        out.push_str("                    }\n");
                        out.push_str("                } else if (part is Map<*, *>) {\n");
                        out.push_str("                    @Suppress(\"UNCHECKED_CAST\")\n");
                        out.push_str("                    val partMap = part as? Map<String, Any>\n");
                        out.push_str("                    if (partMap?.get(\"type\") == \"text\") {\n");
                        out.push_str("                        val textValue = partMap[\"text\"]\n");
                        out.push_str("                        if (textValue != null) {\n");
                        out.push_str("                            sb.append(textValue.toString())\n");
                        out.push_str("                        }\n");
                        out.push_str("                    }\n");
                        out.push_str("                }\n");
                        out.push_str("            }\n");
                        out.push_str("            sb.toString()\n");
                        out.push_str("        }\n");
                    } else {
                        out.push_str("        is ");
                        out.push_str(name);
                        out.push('.');
                        out.push_str(&variant.name);
                        out.push_str(" -> \"\"\n");
                    }
                }
                _ => {
                    out.push_str("        is ");
                    out.push_str(name);
                    out.push('.');
                    out.push_str(&variant.name);
                    out.push_str(" -> \"\"\n");
                }
            }
        } else {
            out.push_str("        is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> \"\"\n");
        }
    }

    out.push_str("    }\n");
}

/// Emit a Jackson `StdDeserializer` for an untagged (`#[serde(untagged)]`) sealed
/// class.  The deserializer inspects the JSON node kind and tries variants in order.
pub(super) fn emit_kotlin_untagged_deserializer(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Deserializer : com.fasterxml.jackson.databind.deser.std.StdDeserializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.JsonNode>(parser)\n");

    for variant in &en.variants {
        if variant.fields.is_empty() {
            continue;
        }

        let (condition, inner_expr) = if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            let ty = &variant.fields[0].ty;
            match ty {
                TypeRef::String => ("node.isTextual", format!("{name}.{}(node.asText())", variant.name)),
                TypeRef::Vec(elem_ty) => {
                    let elem_class = super::kotlin_class_name_for_type(elem_ty);
                    let expr = format!(
                        "run {{\n                val javaType = ctx.typeFactory.constructCollectionType(List::class.java, {elem_class}::class.java)\n                @Suppress(\"UNCHECKED_CAST\")\n                {name}.{}(ctx.readTreeAsValue<List<{elem_class}>>(node, javaType) as List<{elem_class}>)\n            }}",
                        variant.name,
                    );
                    ("node.isArray", expr)
                }
                TypeRef::Primitive(_) => {
                    let class_name = super::kotlin_class_name_for_type(ty);
                    (
                        "node.isNumber",
                        format!(
                            "{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))",
                            variant.name
                        ),
                    )
                }
                TypeRef::Named(n) => (
                    "true",
                    format!(
                        "try {{ {name}.{}(ctx.readTreeAsValue(node, {n}::class.java)) }} catch (_: com.fasterxml.jackson.databind.exc.MismatchedInputException) {{ null as? {name} }} catch (_: com.fasterxml.jackson.databind.exc.UnrecognizedPropertyException) {{ null as? {name} }}",
                        variant.name
                    ),
                ),
                _ => {
                    let class_name = super::kotlin_class_name_for_type(ty);
                    (
                        "node.isObject",
                        format!(
                            "{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))",
                            variant.name
                        ),
                    )
                }
            }
        } else {
            let struct_class = format!("{name}.{}", variant.name);
            (
                "node.isObject",
                format!("ctx.readTreeAsValue<{struct_class}>(node, {struct_class}::class.java)"),
            )
        };

        out.push_str("        if (");
        out.push_str(condition);
        out.push_str(") ");
        if condition == "true" && inner_expr.contains("try {") {
            out.push_str("{\n");
            out.push_str("            val result = ");
            out.push_str(&inner_expr);
            out.push('\n');
            out.push_str("            if (result != null) return result\n");
            out.push_str("        }\n");
        } else {
            out.push_str("return ");
            out.push_str(&inner_expr);
            out.push('\n');
        }
    }

    out.push_str("        throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("            parser, \"Cannot deserialize ");
    out.push_str(name);
    out.push_str(": no matching variant for JSON shape\", null, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("        )\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdSerializer` for an untagged (`#[serde(untagged)]`) sealed
/// class.  Each variant serializes as its inner value (for newtype variants) or
/// as a plain JSON object (for struct variants).
///
/// Without this serializer, Jackson would emit `{"field0": "..."}` for a newtype
/// variant like `UserContent.Text(field0: String)`, but Rust expects just `"..."`.
pub(super) fn emit_kotlin_untagged_serializer(out: &mut String, en: &EnumDef) {
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
        if variant.fields.is_empty() {
            out.push_str("            is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> gen.writeNull()\n");
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
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(elem_type) = inner.as_ref() {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "sealed_vec_serializer_block.jinja",
                        minijinja::context! {
                            enum_name => name,
                            variant_name => variant.name,
                            elem_type => elem_type,
                            field_name => field_name,
                        },
                    ));
                } else {
                    out.push_str("            is ");
                    out.push_str(name);
                    out.push('.');
                    out.push_str(&variant.name);
                    out.push_str(" -> mapper.writeValue(gen, value.");
                    out.push_str(&field_name);
                    out.push_str(")\n");
                }
            } else {
                out.push_str("            is ");
                out.push_str(name);
                out.push('.');
                out.push_str(&variant.name);
                out.push_str(" -> mapper.writeValue(gen, value.");
                out.push_str(&field_name);
                out.push_str(")\n");
            }
        } else {
            out.push_str("            is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> mapper.writeValue(gen, value as ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(")\n");
        }
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}
