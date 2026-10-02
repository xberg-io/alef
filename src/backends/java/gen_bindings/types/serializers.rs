use crate::backends::java::type_map::java_type;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{EnumDef, TypeRef};

use crate::backends::java::gen_bindings::helpers::is_tuple_field_name;
use crate::codegen::java_literal::{escape_java_comment_text, escape_java_string_literal};
use crate::codegen::naming::wire_variant_value;
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};

/// True when serde writes only the tag for this variant, with no payload of any kind.
fn is_unit_variant(variant: &crate::core::ir::EnumVariant) -> bool {
    let is_newtype = variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name);
    variant.fields.is_empty() || (is_newtype && matches!(&variant.fields[0].ty, crate::core::ir::TypeRef::Unit))
}

/// The shape of one variant as both Jackson codecs need to see it: its wire discriminator plus
/// which of serde's three payload forms (unit, newtype, struct) it carries.
///
/// `variants` is passed in rather than read off `enum_def` because the two codecs disagree about
/// binding-excluded variants: the deserializer must not offer a `case` for a variant the binding
/// cannot construct, while the serializer still needs its `instanceof` arm. ~keep
fn variant_contexts<'a>(
    enum_def: &EnumDef,
    variants: impl Iterator<Item = &'a crate::core::ir::EnumVariant>,
) -> Vec<minijinja::Value> {
    variants
        .map(|variant| {
            let discriminator = escape_java_string_literal(&wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            ));
            let is_newtype = variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name);
            let is_unit = is_unit_variant(variant);
            let is_tuple = is_newtype && !is_unit;
            let inner_type = if is_tuple {
                java_type(&variant.fields[0].ty).into_owned()
            } else {
                String::new()
            };
            minijinja::context! {
                name => &variant.name,
                discriminator => discriminator,
                is_unit => is_unit,
                is_tuple => is_tuple,
                is_untagged => variant.serde_untagged,
                inner_type => inner_type,
            }
        })
        .collect()
}

pub(crate) fn gen_byte_array_serializer(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.core.JsonGenerator",
        "com.fasterxml.jackson.databind.SerializerProvider",
        "com.fasterxml.jackson.databind.JsonSerializer",
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

/// Generate `DurationMillisSerializer.java`: converts the ergonomic millisecond `Long`
/// used for Rust `Duration` fields into the `{"secs":<u64>,"nanos":<u32>}` object shape
/// `std::time::Duration`'s serde derive actually produces. See
/// `duration_millis_serializer.jinja`.
pub(crate) fn gen_duration_millis_serializer(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.core.JsonGenerator",
        "com.fasterxml.jackson.databind.SerializerProvider",
        "com.fasterxml.jackson.databind.JsonSerializer",
    ];
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&crate::backends::java::template_env::render(
        "duration_millis_serializer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate `DurationMillisDeserializer.java`: the inverse of
/// [`gen_duration_millis_serializer`]. See `duration_millis_deserializer.jinja`.
pub(crate) fn gen_duration_millis_deserializer(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.core.JsonParser",
        "com.fasterxml.jackson.databind.DeserializationContext",
        "com.fasterxml.jackson.databind.JsonDeserializer",
        "com.fasterxml.jackson.databind.JsonNode",
    ];
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&crate::backends::java::template_env::render(
        "duration_millis_deserializer.jinja",
        minijinja::context! {},
    ));
    out
}

/// The tag key the Jackson codecs write, with the fallback used when an enum reaches this path
/// without a `#[serde(tag = ...)]` of its own.
fn tag_field_of(repr: &SerdeEnumRepr) -> &str {
    repr.tag().unwrap_or(DEFAULT_TAG_FIELD)
}

/// Jackson's `@JsonTypeInfo` default property name, kept for enums that reach the union path
/// without declaring a serde tag.
pub(super) const DEFAULT_TAG_FIELD: &str = "type";

/// Emit the Jackson deserializer for a sealed-interface union.
///
/// serde's *internal* form (`#[serde(tag = "role")]`) puts the payload's fields flat beside the
/// tag, so the tag is stripped and the remainder read as the variant. serde's *adjacent* form
/// (`#[serde(tag, content)]`) puts the whole payload — object or scalar — under the content key,
/// so the payload is read from that key instead. Which one applies is decided by
/// [`serde_enum_repr`], never re-derived here. See `sealed_union_deserializer.jinja`.
pub(super) fn gen_sealed_union_deserializer(out: &mut String, _package: &str, enum_def: &EnumDef) {
    let repr = serde_enum_repr(enum_def);
    let variants = variant_contexts(enum_def, enum_def.variants.iter().filter(|v| !v.binding_excluded));
    let needs_content = enum_def
        .variants
        .iter()
        .any(|variant| !variant.binding_excluded && !is_unit_variant(variant));
    let excluded_variants: Vec<String> = enum_def
        .excluded_variants
        .iter()
        .map(|v| {
            escape_java_string_literal(&wire_variant_value(
                &v.name,
                v.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            ))
        })
        .collect();
    out.push_str(&crate::backends::java::template_env::render(
        "sealed_union_deserializer.jinja",
        minijinja::context! {
            class_name => &enum_def.name,
            tag_field => escape_java_string_literal(tag_field_of(&repr)),
            content_field => repr.content().map(escape_java_string_literal),
            content_field_doc => repr.content().map(escape_java_comment_text),
            is_adjacent => repr.content().is_some(),
            is_external => matches!(repr, SerdeEnumRepr::External),
            has_external_object_variant => enum_def.variants.iter().any(|variant| {
                !variant.binding_excluded && !variant.serde_untagged && !is_unit_variant(variant)
            }),
            has_untagged_string_variant => enum_def.variants.iter().any(|variant| {
                variant.serde_untagged
                    && variant.fields.len() == 1
                    && is_tuple_field_name(&variant.fields[0].name)
                    && matches!(variant.fields[0].ty, TypeRef::String)
            }),
            needs_content => needs_content,
            variants => variants,
            excluded_variants => excluded_variants,
        },
    ));
}

/// Emit the companion serializer that mirrors `gen_sealed_union_deserializer`.
///
/// For an internally-tagged enum like `#[serde(tag = "role")] enum Message { User(UserMessage), ... }`,
/// the deserializer reads the `role` field, strips it, and dispatches to the matching variant.
/// The serializer must do the inverse: emit a flat object containing the tag field plus the
/// inner record's fields. Without this, Jackson's default serialization wraps the inner value
/// (e.g. `{"value": {...UserMessage...}}`) and Rust's serde rejects the missing tag.
///
/// For an adjacently tagged enum the payload instead goes whole under the content key. Flattening
/// it there would be wrong twice over: a scalar payload has no fields to flatten and was dropped
/// outright, and Rust rejects the flat shape with `expected adjacently tagged enum`. ~keep
pub(super) fn gen_sealed_union_serializer(out: &mut String, _package: &str, enum_def: &EnumDef) {
    let repr = serde_enum_repr(enum_def);
    let variants = variant_contexts(enum_def, enum_def.variants.iter());
    out.push_str(&crate::backends::java::template_env::render(
        "sealed_union_serializer.jinja",
        minijinja::context! {
            class_name => &enum_def.name,
            tag_field => escape_java_string_literal(tag_field_of(&repr)),
            tag_field_doc => escape_java_comment_text(tag_field_of(&repr)),
            content_field => repr.content().map(escape_java_string_literal),
            content_field_doc => repr.content().map(escape_java_comment_text),
            is_adjacent => repr.content().is_some(),
            is_external => matches!(repr, SerdeEnumRepr::External),
            has_untagged_string_variant => enum_def.variants.iter().any(|variant| {
                variant.serde_untagged
                    && variant.fields.len() == 1
                    && is_tuple_field_name(&variant.fields[0].name)
                    && matches!(variant.fields[0].ty, TypeRef::String)
            }),
            variants => variants,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::{gen_duration_millis_deserializer, gen_duration_millis_serializer};

    #[test]
    fn duration_millis_serializer_writes_the_real_duration_wire_shape() {
        let out = gen_duration_millis_serializer("dev.sample_crate");
        assert!(out.contains("class DurationMillisSerializer extends JsonSerializer<Long>"));
        assert!(out.contains("gen.writeNumberField(\"secs\", value / 1000L)"));
        assert!(out.contains("gen.writeNumberField(\"nanos\", (int) ((value % 1000L) * 1_000_000L))"));
        assert!(out.contains("package dev.sample_crate;"));
    }

    #[test]
    fn duration_millis_deserializer_reads_the_real_duration_wire_shape() {
        let out = gen_duration_millis_deserializer("dev.sample_crate");
        assert!(out.contains("class DurationMillisDeserializer extends JsonDeserializer<Long>"));
        assert!(out.contains("node.get(\"secs\")"));
        assert!(out.contains("node.get(\"nanos\")"));
        assert!(out.contains("(secs * 1000L) + (nanos / 1_000_000L)"));
        assert!(out.contains("package dev.sample_crate;"));
    }
}
