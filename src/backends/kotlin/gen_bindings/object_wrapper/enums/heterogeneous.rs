use crate::core::ir::EnumDef;

use super::super::types::{escape_kotlin_string, primitive_type_name};
use super::is_tuple_field_name;
use super::runtime_typed::{self, PAYLOAD_INDENT};
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
            let payload_expr = format!("value.{field_name}");
            if !runtime_typed::try_emit_runtime_typed_payload_write(
                out,
                PAYLOAD_INDENT,
                &payload_expr,
                &field.ty,
                field.optional,
            ) {
                out.push_str(PAYLOAD_INDENT);
                out.push_str("mapper.writeValue(gen, ");
                out.push_str(&payload_expr);
                out.push_str(")\n");
            }
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
            serde_untagged: false,
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

    /// An externally tagged enum mixing a unit variant with a newtype variant carrying `payload`.
    fn heterogeneous_enum(payload: TypeRef) -> EnumDef {
        EnumDef {
            name: "Shape".to_string(),
            rust_path: "crate::Shape".to_string(),
            original_rust_path: "crate::Shape".to_string(),
            variants: vec![
                make_variant("Empty", vec![]),
                make_variant("Filled", vec![make_field("_0", payload)]),
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

    fn mixed_unit_and_data_enum() -> EnumDef {
        heterogeneous_enum(TypeRef::Primitive(crate::core::ir::PrimitiveType::F64))
    }

    fn sealed_vec_payload() -> TypeRef {
        TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string())))
    }

    fn serialize_payload(payload: TypeRef) -> String {
        let en = heterogeneous_enum(payload);
        let mut out = String::new();
        emit_kotlin_heterogeneous_default_serializer(&mut out, &en);
        out
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

    /// Regression: a heterogeneous (externally tagged) newtype payload that is a collection of a
    /// sealed type must be written element by element through the type-wrapping serializer resolved
    /// from each element's RUNTIME class.
    ///
    /// `mapper.writeValue(gen, value.payload)` hands Jackson a value with no static type, so the
    /// collection's elements are resolved dynamically via `findValueSerializer` and written with
    /// `serialize` rather than `serializeWithType`. `@JsonTypeInfo` writes its discriminator only
    /// from `serializeWithType`, and a custom base serializer is cancelled on struct-variant
    /// subclasses with `JsonSerializer.None`, so either way the elements go out untagged and Rust
    /// rejects the array. An external representation is never declaratively polymorphic
    /// (`uses_declarative_polymorphism` requires internal tagging), so this emitter is the only
    /// serializer such an enum ever gets — there is no annotation path to fall back on. Fixed in
    /// the untagged sibling emitter first; this is the same defect on the externally tagged path.
    #[test]
    fn should_type_sealed_vec_payload_elements_by_runtime_class() {
        let out = serialize_payload(sealed_vec_payload());

        assert!(
            out.contains(
                "provider.findTypedValueSerializer(elem.javaClass, true, null).serialize(elem, gen, provider)"
            ),
            "a collection payload must resolve a TYPE-WRAPPING serializer from each element's runtime class; \
             got:\n{out}"
        );
        assert!(
            !out.contains("mapper.writeValue(gen, value.value)"),
            "the erased mapper.writeValue form drops the element discriminator; got:\n{out}"
        );
        assert!(
            !out.contains("findValueSerializer("),
            "an untyped findValueSerializer lookup drops the discriminator on every element; got:\n{out}"
        );
        assert!(
            !out.contains("findTypedValueSerializer(Node::class.java"),
            "the typed lookup must be given the element's RUNTIME class, not the declared base class \
             (the base class emits the tag but drops the payload); got:\n{out}"
        );
    }

    /// The runtime-typed element write must stay *inside* the `{"Variant": ...}` envelope that makes
    /// this the externally tagged form: the array replaces the payload, not the wrapper.
    #[test]
    fn should_keep_the_external_tag_envelope_around_a_collection_payload() {
        let out = serialize_payload(sealed_vec_payload());

        assert!(
            out.contains("                gen.writeFieldName(\"Filled\")\n                gen.writeStartArray()\n"),
            "the array must open directly under the variant's field name; got:\n{out}"
        );
        assert!(
            out.contains("                gen.writeEndArray()\n                gen.writeEndObject()\n"),
            "the single-key object must close after the array; got:\n{out}"
        );
    }

    /// A payload with no `Named` inside cannot lose a discriminator, so it must keep using
    /// `mapper.writeValue` — the specialized path is not a blanket rewrite of every payload.
    #[test]
    fn should_leave_a_payload_without_a_named_type_on_the_mapper_path() {
        let out = serialize_payload(TypeRef::Vec(Box::new(TypeRef::String)));

        assert!(
            out.contains("                mapper.writeValue(gen, value.value)\n"),
            "a List<String> payload has nothing to type by runtime class; got:\n{out}"
        );
        assert!(
            !out.contains("findTypedValueSerializer("),
            "no runtime-typed lookup should be emitted for a payload without a Named type; got:\n{out}"
        );
    }

    /// The remaining container shapes reachable here. `TypeRef` has exactly three container
    /// constructors (`Optional`, `Vec`, `Map`), so this is the complete enumeration of payloads that
    /// can hide a `Named` element behind an erased `mapper.writeValue`.
    #[test]
    fn should_type_every_container_payload_shape_that_hides_a_named_element() {
        let node = || TypeRef::Named("Node".to_string());
        let shapes = [
            TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(node())))),
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(node())))),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(node())),
        ];
        for shape in shapes {
            let out = serialize_payload(shape.clone());
            assert!(
                out.contains("findTypedValueSerializer("),
                "{shape:?} must reach the runtime-typed element write; got:\n{out}"
            );
            assert!(
                !out.contains("findTypedValueSerializer(Node::class.java"),
                "{shape:?} must resolve the RUNTIME class, not the declared base; got:\n{out}"
            );
            assert!(
                !out.contains("mapper.writeValue(gen, value.value)"),
                "{shape:?} must not fall through to the erased mapper.writeValue; got:\n{out}"
            );
        }
    }
}
