use crate::core::ir::{EnumDef, TypeRef};

use super::types::{
    KTFMT_LINE_WIDTH, escape_kotlin_string, fits_single_line, kotlin_type_disambiguated, primitive_type_name,
};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::{kotlin_field_name_with_type, to_screaming_snake};
use crate::codegen::naming::wire_variant_value;

mod heterogeneous;
mod tagged;
mod untagged;

use heterogeneous::{emit_kotlin_heterogeneous_default_deserializer, emit_kotlin_heterogeneous_default_serializer};
use tagged::{emit_kotlin_tagged_deserializer, emit_kotlin_tagged_serializer};
use untagged::{emit_kotlin_untagged_deserializer, emit_kotlin_untagged_serializer};

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String, package: &str) {
    emit_cleaned_kdoc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "enum_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            // Emit per-variant KDoc above the enum constant. Indent matches
            // the template's 4-space lead.
            emit_cleaned_kdoc(out, &en.variants[idx].doc, "    ");
            // When the Rust serde discriminator differs from the Kotlin
            // `SCREAMING_SNAKE_CASE` constant, emit a `@JsonProperty` so
            // Jackson maps the wire value to the right constant on
            // deserialize and back on serialize. This is the typical case
            // when the Rust source uses `#[serde(rename_all = "snake_case")]`
            // or per-variant `#[serde(rename = "...")]`.
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let comma = if idx + 1 == names.len() { ";" } else { "," };

            if discriminator != *name {
                // Format: annotation + variant, optionally on a single line if it fits
                let annotation = format!(
                    "@com.fasterxml.jackson.annotation.JsonProperty(\"{}\")",
                    escape_kotlin_string(&discriminator)
                );
                let variant_line = format!("{}{}", name, comma);
                let total_length = 4 + annotation.len() + 1 + variant_line.len(); // 4 indent, space sep

                if total_length <= KTFMT_LINE_WIDTH {
                    // Fit on single line: "    @annotation VariantName,"
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "enum_json_property_variant_inline.jinja",
                        minijinja::context! {
                            annotation => annotation,
                            variant_line => variant_line,
                        },
                    ));
                } else {
                    // Multi-line: annotation on one line, variant on the next
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "enum_json_property_variant_multiline.jinja",
                        minijinja::context! {
                            annotation => annotation,
                            variant_line => variant_line,
                        },
                    ));
                }
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "enum_variant.jinja",
                    minijinja::context! {
                        name => name,
                        comma => comma,
                    },
                ));
            }
        }

        // Emit @JsonValue method for serialization
        // ktfmt wants "when" on a new line for expression-bodied functions, even if it would fit
        out.push_str("\n    @com.fasterxml.jackson.annotation.JsonValue\n");
        out.push_str("    fun toWire(): String =\n");
        out.push_str("        when (this) {\n");
        for (idx, name) in names.iter().enumerate() {
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            out.push_str("            ");
            out.push_str(name);
            out.push_str(" -> \"");
            out.push_str(&escape_kotlin_string(&discriminator));
            out.push_str("\"\n");
        }
        out.push_str("        }\n");

        // Emit @JsonCreator companion object method for deserialization
        out.push_str("\n    companion object {\n");
        out.push_str("        @com.fasterxml.jackson.annotation.JsonCreator\n");
        out.push_str("        @JvmStatic\n");
        out.push_str("        fun fromWire(value: String): ");
        out.push_str(&en.name);
        out.push_str(" =\n");
        out.push_str("            when (value) {\n");
        for (idx, name) in names.iter().enumerate() {
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let discriminator_lower = discriminator.to_lowercase();
            if discriminator != discriminator_lower {
                // Accept both the serde-renamed wire form (e.g. "Angle") and its lowercase
                // variant (e.g. "angle"). Some core enums implement Serialize/Deserialize
                // manually via a token normaliser (see UrlEscapeStyle), so the wire form on
                // the JSON boundary may be lowercase even when alef's IR sees the raw
                // PascalCase variant name. Matching both keeps the binding robust against
                // either convention without forcing the core to add #[serde(rename_all)].
                // Emit each match value on its own line per ktfmt's multi-value arm formatting
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "enum_wire_multivalue_arm.jinja",
                    minijinja::context! {
                        discriminator => escape_kotlin_string(&discriminator),
                        discriminator_lower => escape_kotlin_string(&discriminator_lower),
                        name => name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "enum_wire_arm.jinja",
                    minijinja::context! {
                        discriminator => escape_kotlin_string(&discriminator),
                        name => name,
                    },
                ));
            }
        }
        out.push_str("                else -> throw IllegalArgumentException(\"Unknown ");
        out.push_str(&en.name);
        out.push_str(" value: $value\")\n");
        out.push_str("            }\n");
        out.push_str("    }\n");

        out.push_str("}\n");
    } else {
        // Sealed classes with data variants need a Jackson custom deserializer so that
        // Jackson (used by e2e tests via ObjectMapper) can reconstruct the correct
        // subtype.  Unit-only sealed classes use a simple `when` dispatch and do not
        // need deserialization support.
        //
        // Default serde encoding (no `#[serde(tag)]` and no `#[serde(untagged)]`) on
        // a sealed-class enum that mixes unit and data variants produces an
        // externally-tagged HETEROGENEOUS wire format: unit variants serialize as a
        // bare string `"Variant"`, data variants serialize as an object
        // `{"Variant": <inner>}`. Jackson's built-in sealed-class default cannot
        // round-trip that shape, so we emit a heterogeneous-default (de)serializer
        // pair here too. This is the common case for Rust enums that grew an
        // `Other(String)` newtype catch-all variant alongside named unit variants.
        let has_unit_variant = en.variants.iter().any(|v| v.fields.is_empty());
        let has_data_variant = en.variants.iter().any(|v| !v.fields.is_empty());
        let needs_heterogeneous_default =
            has_unit_variant && has_data_variant && en.serde_tag.is_none() && !en.serde_untagged;
        let needs_deserializer = en.serde_tag.is_some() || en.serde_untagged || needs_heterogeneous_default;
        if needs_deserializer {
            out.push_str("@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = ");
            out.push_str(&en.name);
            out.push_str("Deserializer::class)\n");
        }
        // Sealed classes need custom serializers so that round-trip
        // (Kotlin → JSON → Rust) works correctly.
        // - Tagged: the tag field must be injected into the JSON output.
        // - Untagged: newtype variants must serialize as their inner value,
        //   not as a data-class wrapper object.
        // - Heterogeneous default: unit variants become bare strings, data variants
        //   become `{"Variant": <inner>}` — Jackson's default sealed-class serializer
        //   cannot emit that mixed shape.
        let needs_serializer = en.serde_tag.is_some() || en.serde_untagged || needs_heterogeneous_default;
        if needs_serializer {
            out.push_str("@com.fasterxml.jackson.databind.annotation.JsonSerialize(using = ");
            out.push_str(&en.name);
            out.push_str("Serializer::class)\n");
        }
        out.push_str(&crate::backends::kotlin::template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));

        // Collect all variant names so we can detect name-shadowing in field types.
        // Inside a sealed class body, a nested data class `Foo` shadows any outer
        // `Foo` with the same simple name.  When a field type has the same name as a
        // sibling variant we must fully-qualify the field type with the package path
        // to avoid the compiler resolving the type to the variant itself (Bug E).
        let variant_names: std::collections::HashSet<&str> = en.variants.iter().map(|v| v.name.as_str()).collect();

        for variant in &en.variants {
            // Sealed-class variants render their rustdoc above the nested
            // object/data class declaration.
            emit_cleaned_kdoc(out, &variant.doc, "    ");
            if variant.fields.is_empty() {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                // Newtype/tuple variants (a single tuple-named field wrapping an
                // inner type, e.g. `data class User(val message: UserMessage)`)
                // do NOT need the inherited annotation reset:
                //   - The parent serializer routes via `value.<inner>` (e.g.
                //     `mapper.valueToTree(value.message)`), so the type Jackson
                //     resolves the serializer for is the INNER non-sealed class
                //     — no recursion is possible.
                //   - The parent deserializer routes via
                //     `ctx.readTreeAsValue<Inner>(payload, Inner::class.java)`,
                //     reading into the inner non-sealed class — no recursion.
                //
                // Emitting `@JsonSerialize(using = None::class)` on newtype
                // variants is in fact HARMFUL: when Jackson encounters a value
                // of runtime type `Sealed.Variant`, the variant-level reset
                // annotation defeats the parent's custom serializer entirely,
                // so the value is emitted as a default POJO `{"<field>":...}`
                // instead of the discriminator-flattened form
                // (`{"role":"user",...}` for tagged sealed classes, or just the
                // inner value for untagged ones).
                //
                // Named-field struct variants (variants carrying their own
                // named fields directly) DO need the reset: the parent
                // (de)serializer routes via `value as Sealed.Variant` or
                // `readTreeAsValue<Variant>(...)`, both of which target the
                // variant subtype — inheriting the parent's custom annotation
                // would loop back into the parent (de)serializer.
                let is_newtype_variant = variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name);
                let emit_reset = !is_newtype_variant;
                if needs_deserializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n");
                }
                if needs_serializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n");
                }

                // Pre-build field strings for the ktfmt single-line heuristic.
                // Annotations force multi-line because they cannot be inlined.
                let has_annotations = (needs_deserializer || needs_serializer) && emit_reset;
                let mut variant_field_strings: Vec<String> = Vec::with_capacity(variant.fields.len());
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = kotlin_type_disambiguated(&f.ty, f.optional, &variant_names, package);
                    let field_type_name = match &f.ty {
                        TypeRef::Named(name) => Some(name.as_str()),
                        TypeRef::String => Some("String"),
                        TypeRef::Primitive(p) => Some(primitive_type_name(p)),
                        _ => None,
                    };
                    let name =
                        kotlin_field_name_with_type(&f.name, idx, field_type_name, &variant.name, variant.fields.len());
                    variant_field_strings.push(format!("val {name}: {ty_str}"));
                }

                let variant_prefix = format!("data class {}", variant.name);
                let variant_suffix = format!(" : {}()", en.name);
                let use_single_line = !has_annotations
                    && fits_single_line("    ", &variant_prefix, &variant_field_strings, &variant_suffix);

                if use_single_line {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "sealed_variant_inline.jinja",
                        minijinja::context! {
                            variant_prefix => variant_prefix,
                            fields => variant_field_strings.join(", "),
                            variant_suffix => variant_suffix,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "sealed_variant_header.jinja",
                        minijinja::context! {
                            variant_prefix => variant_prefix,
                        },
                    ));
                    for field_str in &variant_field_strings {
                        out.push_str(&crate::backends::kotlin::template_env::render(
                            "sealed_variant_field.jinja",
                            minijinja::context! {
                                field => field_str,
                            },
                        ));
                    }
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "sealed_variant_close.jinja",
                        minijinja::context! {
                            variant_suffix => variant_suffix,
                        },
                    ));
                }
            }
        }
        out.push_str("}\n");

        // Emit the custom Jackson deserializer immediately after the sealed class.
        if needs_deserializer {
            if let Some(tag_field) = &en.serde_tag {
                emit_kotlin_tagged_deserializer(out, en, tag_field);
            } else if en.serde_untagged {
                emit_kotlin_untagged_deserializer(out, en);
            } else if needs_heterogeneous_default {
                emit_kotlin_heterogeneous_default_deserializer(out, en);
            }
        }
        // Emit the custom Jackson serializer for tagged/untagged/heterogeneous-default
        // sealed classes so that round-trip (Kotlin → JSON → Rust) works correctly.
        if let Some(tag_field) = &en.serde_tag {
            emit_kotlin_tagged_serializer(out, en, tag_field);
        } else if en.serde_untagged {
            emit_kotlin_untagged_serializer(out, en);
        } else if needs_heterogeneous_default {
            emit_kotlin_heterogeneous_default_serializer(out, en);
        }
    }
}

/// True when a field's name is a tuple-field index (e.g. `"0"`, `"_0"`).
pub(super) fn is_tuple_field_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
}

/// Return the simple Kotlin class name that Jackson can deserialise a TypeRef into
/// using `readTreeAsValue(node, <name>::class.java)`.
/// For user-defined Named types it is the short class name (same package, no import needed).
pub(super) fn kotlin_class_name_for_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean".to_string(),
                PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_string(),
                PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_string(),
                PrimitiveType::U32 | PrimitiveType::I32 => "Int".to_string(),
                PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                    "Long".to_string()
                }
                PrimitiveType::F32 => "Float".to_string(),
                PrimitiveType::F64 => "Double".to_string(),
            }
        }
        TypeRef::Named(n) => n.clone(),
        TypeRef::Vec(_) => "List".to_string(),
        TypeRef::Map(_, _) => "Map".to_string(),
        _ => "Any".to_string(),
    }
}
