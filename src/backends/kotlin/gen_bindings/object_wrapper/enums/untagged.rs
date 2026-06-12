use crate::core::ir::{EnumDef, TypeRef};

use super::super::types::primitive_type_name;
use super::is_tuple_field_name;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type;

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
    // Suppress detekt LongMethod: the number of if-branches scales with the number
    // of variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
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
            // Unit variant in an untagged enum — skip shape-based dispatch; cannot match.
            continue;
        }

        // Determine what JSON shape this variant expects based on its first field.
        let (condition, inner_expr) = if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Tuple/newtype variant — the JSON IS the inner value.
            let ty = &variant.fields[0].ty;
            match ty {
                TypeRef::String => ("node.isTextual", format!("{name}.{}(node.asText())", variant.name)),
                TypeRef::Vec(elem_ty) => {
                    // Use JavaType to carry the generic element type so Jackson can
                    // construct a properly-typed List<T> rather than a raw List<*>.
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
                TypeRef::Named(n) => {
                    // Named types can be either enums (stringify via @JsonValue + @JsonCreator)
                    // or structs (objectify). Without enum type information in the backend,
                    // we conservatively check node.isObject for struct variants and fall through
                    // to a catch-all deserialization that handles both cases at the end.
                    // For now, we'll check for both textual and object nodes to support both.
                    (
                        "true", // Try all Named types; let deserialization determine success
                        format!(
                            "try {{ {name}.{}(ctx.readTreeAsValue(node, {n}::class.java)) }} catch (_: com.fasterxml.jackson.databind.exc.MismatchedInputException) {{ null as? {name} }} catch (_: com.fasterxml.jackson.databind.exc.UnrecognizedPropertyException) {{ null as? {name} }}",
                            variant.name
                        ),
                    )
                }
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
            // Struct variant with named fields — JSON must be an object.
            // `readTreeAsValue` returns the correct data class subtype directly;
            // no variant-constructor wrapping needed.
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
            // For try-catch branches, only return if result is not null
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
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str("        val mapper = (gen.codec as? com.fasterxml.jackson.databind.ObjectMapper) ?: com.fasterxml.jackson.databind.ObjectMapper().findAndRegisterModules()\n");
    out.push_str("        when (value) {\n");

    for variant in &en.variants {
        if variant.fields.is_empty() {
            // Unit variant in an untagged enum: emit null (safest fallback).
            out.push_str("            is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> gen.writeNull()\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: serialize the inner value directly
            // (not wrapped in an object), matching serde's untagged behaviour.
            // Use the same payload-derived field name that the data-class declaration
            // uses (via kotlin_field_name_with_type), so `value.<field>` resolves.
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
            // When the inner type is Vec<SealedClass>, mapper.writeValue dispatches to
            // each element's runtime-subtype serializer (which has @JsonSerialize reset
            // to None), losing the sealed-class "type" discriminator. Use
            // provider.findValueSerializer on the declared element type instead so the
            // sealed-class serializer (which writes "type") is always called.
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
            // Named-field struct variant: cast to the concrete variant type before
            // serializing so Jackson resolves the serializer against the variant
            // class (which has @JsonSerialize reset to the default POJO serializer),
            // not against the parent sealed class (which would recurse infinitely).
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
