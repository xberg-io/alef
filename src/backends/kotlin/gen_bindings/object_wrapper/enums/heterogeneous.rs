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
    // `properties()` (Jackson 2.15+) replaces the deprecated (since 2.19) `fields()`; its ~keep
    // `Set<Map.Entry<...>>` return type still needs `.iterator()` to keep the identical ~keep
    // hasNext()/next() single-entry-object shape below. ~keep
    out.push_str("            val it = obj.properties().iterator()\n");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef};

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            version: Default::default(),
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }
    }

    fn mixed_unit_and_data_enum() -> EnumDef {
        EnumDef {
            name: "Shape".to_string(),
            rust_path: "crate::Shape".to_string(),
            original_rust_path: "crate::Shape".to_string(),
            variants: vec![
                make_variant("Empty", vec![]),
                make_variant(
                    "Circle",
                    vec![make_field(
                        "_0",
                        TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
                    )],
                ),
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_content: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
            has_default: false,
        }
    }

    /// Regression: Jackson deprecated `JsonNode.fields()` (returning an `Iterator`) in favor
    /// of `properties()` (returning a `Set`) as of 2.19. The single-entry-object probe below
    /// relies on `Iterator`'s `hasNext()`/`next()`, so the generated deserializer must keep
    /// calling `.iterator()` on the replacement rather than dropping it.
    #[test]
    fn heterogeneous_deserializer_uses_properties_not_the_deprecated_fields_method() {
        let en = mixed_unit_and_data_enum();
        let mut out = String::new();
        emit_kotlin_heterogeneous_default_deserializer(&mut out, &en);

        assert!(
            out.contains("val it = obj.properties().iterator()"),
            "deserializer must read entries via the non-deprecated properties() accessor: {out}"
        );
        assert!(
            !out.contains(".fields()"),
            "deserializer must not call the deprecated (since Jackson 2.19) fields() method: {out}"
        );
    }
}
