use crate::core::ir::{EnumDef, EnumVariant, TypeRef};

use super::super::types::primitive_type_name;
use super::is_tuple_field_name;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type;
use crate::backends::kotlin::template_env::render;
use crate::codegen::naming::wire_variant_value;
use crate::codegen::serde_enum_repr::SerdeEnumRepr;

/// True when serde writes only the tag for this variant, with no payload of any kind.
fn is_unit_variant(variant: &EnumVariant) -> bool {
    variant.fields.is_empty()
}

/// True when the variant is a newtype (`Variant(Inner)`) rather than a struct variant.
fn is_newtype_variant(variant: &EnumVariant) -> bool {
    variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name)
}

/// The Kotlin expression yielding a variant's payload from the `value` being serialized.
///
/// A newtype variant exposes it as the generated property; a struct variant *is* the payload, so
/// the sealed base is narrowed to the variant subclass instead.
fn payload_expression(enum_name: &str, variant: &EnumVariant) -> String {
    if is_newtype_variant(variant) {
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
        format!("value.{field_name}")
    } else {
        format!("value as {enum_name}.{}", variant.name)
    }
}

/// Everything both Jackson codecs need to know about one variant.
fn variant_contexts(en: &EnumDef) -> Vec<minijinja::Value> {
    en.variants
        .iter()
        .map(|variant| {
            let discriminator = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let is_tuple = is_newtype_variant(variant);
            let inner_class = if is_tuple {
                super::kotlin_class_name_for_type(&variant.fields[0].ty)
            } else {
                String::new()
            };
            crate::alef_context! {
                name => &variant.name,
                discriminator => discriminator,
                is_unit => is_unit_variant(variant),
                is_tuple => is_tuple,
                inner_class => inner_class,
                payload_expression => payload_expression(&en.name, variant),
            }
        })
        .collect()
}

/// The template context both codecs share.
///
/// `payload_reference` is how the deserializer names the payload node: the adjacent form reads it
/// lazily through a local function, because a unit variant's document has no content key at all.
fn codec_context(en: &EnumDef, repr: &SerdeEnumRepr) -> minijinja::Value {
    let is_adjacent = repr.content().is_some();
    crate::alef_context! {
        class_name => &en.name,
        tag_field => repr.tag().unwrap_or_default(),
        content_field => repr.content(),
        is_adjacent => is_adjacent,
        needs_content => en.variants.iter().any(|variant| !is_unit_variant(variant)),
        payload_reference => if is_adjacent { "payload()" } else { "payload" },
        variants => variant_contexts(en),
    }
}

/// Emit a Jackson `StdSerializer` for a tagged (`#[serde(tag = ...)]`) sealed class.
///
/// serde's *internal* form carries the payload's fields flat beside the tag, so the payload is
/// serialized to a tree and the tag injected into it. serde's *adjacent* form
/// (`#[serde(tag, content)]`) puts the payload whole under the content key instead — which is also
/// the only form that survives a scalar payload, since the internal path casts the payload tree to
/// `ObjectNode` and a `String` payload is a `TextNode`. Which form applies comes from
/// [`SerdeEnumRepr`], never re-derived here. ~keep
pub(super) fn emit_kotlin_tagged_serializer(out: &mut String, en: &EnumDef, repr: &SerdeEnumRepr) {
    out.push_str(&render("tagged_serializer.jinja", codec_context(en, repr)));
}

/// Emit the Jackson `StdDeserializer` mirroring [`emit_kotlin_tagged_serializer`].
pub(super) fn emit_kotlin_tagged_deserializer(out: &mut String, en: &EnumDef, repr: &SerdeEnumRepr) {
    out.push_str(&render("tagged_deserializer.jinja", codec_context(en, repr)));
}
