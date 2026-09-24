use crate::core::ir::{EnumDef, TypeRef};

use super::types::{
    KTFMT_LINE_WIDTH, escape_kotlin_string, fits_single_line, kotlin_type_disambiguated, primitive_type_name,
};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::{kotlin_field_name_with_type, to_screaming_snake};
use crate::codegen::naming::wire_variant_value;
use crate::codegen::serde_enum_repr::SerdeEnumRepr;

mod heterogeneous;
mod tagged;
mod untagged;

use heterogeneous::{emit_kotlin_heterogeneous_default_deserializer, emit_kotlin_heterogeneous_default_serializer};
use tagged::{emit_kotlin_tagged_deserializer, emit_kotlin_tagged_serializer};
use untagged::{emit_kotlin_untagged_deserializer, emit_kotlin_untagged_serializer};

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String, package: &str, text_types: &[String]) {
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
            emit_cleaned_kdoc(out, &en.variants[idx].doc, "    ");
            // when the Rust source uses `#[serde(rename_all = "snake_case")]`
            // or per-variant `#[serde(rename = "...")]`.
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let comma = if idx + 1 == names.len() { ";" } else { "," };

            if discriminator != *name {
                let annotation = format!(
                    "@com.fasterxml.jackson.annotation.JsonProperty(\"{}\")",
                    escape_kotlin_string(&discriminator)
                );
                let variant_line = format!("{}{}", name, comma);
                let total_length = 4 + annotation.len() + 1 + variant_line.len();

                if total_length <= KTFMT_LINE_WIDTH {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "enum_json_property_variant_inline.jinja",
                        minijinja::context! {
                            annotation => annotation,
                            variant_line => variant_line,
                        },
                    ));
                } else {
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

        // A generated enum stands for a serde wire value, so that is what it must render as.
        // The Kotlin default prints the constant name (`ANCHOR`), which no consumer comparing
        // against the wire form (`anchor`) can match — case-sensitively it never will. Mirrors
        // `toString()` on the Java emitter's enums. ~keep
        out.push_str("\n    override fun toString(): String = toWire()\n");

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
                // either convention without forcing the core to add #[serde(rename_all)].
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
        // Default serde encoding (no `#[serde(tag)]` and no `#[serde(untagged)]`) on
        let has_unit_variant = en.variants.iter().any(|v| v.fields.is_empty());
        let has_data_variant = en.variants.iter().any(|v| !v.fields.is_empty());
        let needs_heterogeneous_default =
            has_unit_variant && has_data_variant && en.serde_tag.is_none() && !en.serde_untagged;
        let repr = crate::codegen::serde_enum_repr::serde_enum_repr(en);
        let declarative = uses_declarative_polymorphism(en, &repr);
        if declarative {
            push_declarative_polymorphism_annotations(out, en, repr.tag().unwrap_or_default());
        }
        let needs_deserializer =
            !declarative && (en.serde_tag.is_some() || en.serde_untagged || needs_heterogeneous_default);
        if needs_deserializer {
            out.push_str("@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = ");
            out.push_str(&en.name);
            out.push_str("Deserializer::class)\n");
        }
        let needs_serializer =
            !declarative && (en.serde_tag.is_some() || en.serde_untagged || needs_heterogeneous_default);
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

        let variant_names: std::collections::HashSet<&str> = en.variants.iter().map(|v| v.name.as_str()).collect();

        for variant in &en.variants {
            emit_cleaned_kdoc(out, &variant.doc, "    ");
            if variant.fields.is_empty() {
                if declarative {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = ");
                    out.push_str(&singleton_deserializer_name(&en.name, &variant.name));
                    out.push_str("::class)\n");
                }
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                let is_newtype_variant = variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name);
                let emit_reset = !is_newtype_variant;
                if needs_deserializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n");
                }
                if needs_serializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n");
                }

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

        let emit_text = en.serde_untagged && text_types.iter().any(|t| t == &en.name);
        if emit_text {
            untagged::emit_kotlin_text_accessor(out, en);
        }

        out.push_str("}\n");

        if declarative {
            for variant in en.variants.iter().filter(|v| v.fields.is_empty()) {
                push_singleton_variant_deserializer(out, &en.name, &variant.name);
            }
            return;
        }

        let tagged_repr = match repr {
            repr @ (SerdeEnumRepr::Internal { .. } | SerdeEnumRepr::Adjacent { .. }) => Some(repr),
            SerdeEnumRepr::External | SerdeEnumRepr::Untagged => None,
        };
        if needs_deserializer {
            if let Some(repr) = &tagged_repr {
                emit_kotlin_tagged_deserializer(out, en, repr);
            } else if en.serde_untagged {
                emit_kotlin_untagged_deserializer(out, en);
            } else if needs_heterogeneous_default {
                emit_kotlin_heterogeneous_default_deserializer(out, en);
            }
        }
        if let Some(repr) = &tagged_repr {
            emit_kotlin_tagged_serializer(out, en, repr);
        } else if en.serde_untagged {
            emit_kotlin_untagged_serializer(out, en);
        } else if needs_heterogeneous_default {
            emit_kotlin_heterogeneous_default_serializer(out, en);
        }
    }
}

/// True when Jackson's declarative polymorphism — `@JsonTypeInfo` + `@JsonSubTypes` — expresses
/// this enum's wire form, which is the model the Java emitter uses.
///
/// The declarative form matters because Jackson resolves a serializer by the value's *runtime*
/// class. A custom serializer on the sealed base has to be cancelled on every subclass
/// (`using = JsonSerializer.None::class`) or it recurses through `valueToTree`, and that reset is
/// exactly what a `List<Base>` erased to `Object` finds — so the tag-writing code never runs.
/// `@JsonTypeInfo` is inherited by the subclasses instead of suppressed, so it fires on runtime
/// dispatch.
///
/// `@JsonTypeInfo(Id.NAME, property = tag)` writes the tag *beside* the payload's fields, which is
/// serde's *internal* tagging and nothing else; and a newtype variant's payload is not a bean with
/// fields to sit beside. Both stay on the hand-written codecs. Same predicate as the Java
/// backend's `needs_unwrapped`. ~keep
fn uses_declarative_polymorphism(en: &EnumDef, repr: &SerdeEnumRepr) -> bool {
    matches!(repr, SerdeEnumRepr::Internal { .. })
        && !en
            .variants
            .iter()
            .any(|v| v.fields.len() == 1 && is_tuple_field_name(&v.fields[0].name))
}

/// [`uses_declarative_polymorphism`] for callers that hold only the enum.
///
/// Lets the DTO emitter skip the `@field:JsonSerialize(`as`/`contentAs` = Sealed::class)`
/// annotation for these types. That annotation exists to pin the *declared* type so the custom
/// base serializer is reached; a declarative type has none, so pinning it would ask Jackson to
/// serialize the field through the abstract base — which has no properties of its own. Java
/// annotates no field this way for the same reason. ~keep
pub(crate) fn enum_uses_declarative_polymorphism(en: &EnumDef) -> bool {
    uses_declarative_polymorphism(en, &crate::codegen::serde_enum_repr::serde_enum_repr(en))
}

fn push_declarative_polymorphism_annotations(out: &mut String, en: &EnumDef, tag: &str) {
    out.push_str("@com.fasterxml.jackson.annotation.JsonTypeInfo(use = ");
    out.push_str("com.fasterxml.jackson.annotation.JsonTypeInfo.Id.NAME, property = \"");
    out.push_str(&escape_kotlin_string(tag));
    out.push_str("\", visible = false)\n");

    let subtypes: Vec<String> = en
        .variants
        .iter()
        .map(|variant| {
            let discriminator = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            format!(
                "    com.fasterxml.jackson.annotation.JsonSubTypes.Type(value = {}.{}::class, name = \"{}\")",
                en.name,
                variant.name,
                escape_kotlin_string(&discriminator)
            )
        })
        .collect();
    out.push_str("@com.fasterxml.jackson.annotation.JsonSubTypes(\n");
    out.push_str(&subtypes.join(",\n"));
    out.push_str("\n)\n");

    out.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
}

fn singleton_deserializer_name(enum_name: &str, variant_name: &str) -> String {
    format!("{enum_name}{variant_name}Deserializer")
}

/// Emit a deserializer that returns a unit variant's singleton rather than a fresh instance.
///
/// A unit variant is a Kotlin `object`, whose equality is identity. Jackson's bean path reaches
/// the private constructor and builds a *second* instance, so `decoded == Enum.Variant` stops
/// holding — a regression against the hand-written codec this path replaces, and one the Java
/// emitter never has because its unit variants are value-equal empty records. ~keep
fn push_singleton_variant_deserializer(out: &mut String, enum_name: &str, variant_name: &str) {
    let class_name = singleton_deserializer_name(enum_name, variant_name);
    out.push_str(&format!(
        "\nprivate class {class_name} : \
         com.fasterxml.jackson.databind.deser.std.StdDeserializer<{enum_name}.{variant_name}>\
         ({enum_name}.{variant_name}::class.java) {{\n\
         \x20   override fun deserialize(\n\
         \x20       parser: com.fasterxml.jackson.core.JsonParser,\n\
         \x20       ctx: com.fasterxml.jackson.databind.DeserializationContext,\n\
         \x20   ): {enum_name}.{variant_name} {{\n\
         \x20       parser.skipChildren()\n\
         \x20       return {enum_name}.{variant_name}\n\
         \x20   }}\n\
         }}\n"
    ));
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
