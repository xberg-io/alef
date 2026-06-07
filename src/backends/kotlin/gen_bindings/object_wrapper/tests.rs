use super::*;
use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, TypeRef};

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_variant(name: &str, serde_rename: Option<&str>, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: serde_rename.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
    }
}

fn make_enum(
    name: &str,
    serde_tag: Option<&str>,
    serde_untagged: bool,
    serde_rename_all: Option<&str>,
    variants: Vec<EnumVariant>,
) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("crate::{name}"),
        original_rust_path: format!("crate::{name}"),
        variants,
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: serde_tag.map(str::to_string),
        serde_untagged,
        serde_rename_all: serde_rename_all.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    }
}

/// Regression: sealed classes with `#[serde(tag = ...)]` must emit
/// `@JsonDeserialize` annotation and a companion deserializer that reads the
/// tag field and dispatches per variant.
#[test]
fn emit_enum_tagged_sealed_class_emits_json_deserialize_annotation() {
    let en = make_enum(
        "Message",
        Some("role"),
        false,
        None,
        vec![
            make_variant(
                "System",
                Some("system"),
                vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()))],
            ),
            make_variant(
                "User",
                Some("user"),
                vec![make_field("_0", TypeRef::Named("UserMessage".to_string()))],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");
    assert!(
        out.contains("@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = MessageDeserializer::class)"),
        "missing @JsonDeserialize annotation on tagged sealed class; got:\n{out}",
    );
    assert!(
        out.contains("private class MessageDeserializer"),
        "missing MessageDeserializer class; got:\n{out}",
    );
    assert!(
        out.contains("node.get(\"role\")"),
        "deserializer must read the 'role' tag field; got:\n{out}",
    );
    assert!(
        out.contains("\"system\" ->"),
        "deserializer must dispatch on variant 'system'; got:\n{out}",
    );
    // Bug D regression: the tag field must be stripped from the payload before
    // passing it to readTreeAsValue.  Inner types (e.g. SystemMessage) do not
    // declare a `role` field, so Jackson rejects the extra key without this fix.
    assert!(
        out.contains("val tag = node.get(\"role\")?.asText()"),
        "tagged deserializer must extract tag into separate variable; got:\n{out}",
    );
    assert!(
            out.contains("val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"role\") }"),
            "tagged deserializer must strip tag field from payload via cast-safe deepCopy; got:\n{out}",
        );
    // Newtype variant: must wrap the inner type in the variant constructor.
    // readTreeAsValue<InnerType> returns the inner type; the variant constructor
    // wraps it to produce the sealed-class value.  Uses `payload` (tag-stripped).
    assert!(
        out.contains("Message.System(ctx.readTreeAsValue<SystemMessage>(payload, SystemMessage::class.java))"),
        "tagged deserializer must wrap readTreeAsValue<InnerType>(payload) in variant constructor for newtype; got:\n{out}",
    );
}

/// Regression: sealed classes with `#[serde(untagged)]` must emit
/// `@JsonDeserialize` annotation and a companion deserializer that tries
/// variants by JSON shape.
#[test]
fn emit_enum_untagged_sealed_class_emits_json_deserialize_annotation() {
    let en = make_enum(
        "EmbeddingInput",
        None,
        true,
        None,
        vec![
            make_variant("Single", None, vec![make_field("_0", TypeRef::String)]),
            make_variant(
                "Multiple",
                None,
                vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");
    assert!(
        out.contains(
            "@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = EmbeddingInputDeserializer::class)"
        ),
        "missing @JsonDeserialize annotation on untagged sealed class; got:\n{out}",
    );
    assert!(
        out.contains("private class EmbeddingInputDeserializer"),
        "missing EmbeddingInputDeserializer class; got:\n{out}",
    );
    assert!(
        out.contains("node.isTextual"),
        "untagged deserializer must check isTextual for String variant; got:\n{out}",
    );
    assert!(
        out.contains("node.isArray"),
        "untagged deserializer must check isArray for List variant; got:\n{out}",
    );
    // Fix: List<T> variants must use JavaType (constructCollectionType) rather
    // than raw List::class.java so Jackson knows the element type.
    assert!(
        out.contains("ctx.typeFactory.constructCollectionType(List::class.java, String::class.java)"),
        "untagged deserializer must use constructCollectionType for List<String> variant; got:\n{out}",
    );
    assert!(
        !out.contains("ctx.readTreeAsValue(node, List::class.java)"),
        "untagged deserializer must NOT use raw List::class.java; got:\n{out}",
    );
}

/// Regression (Bug A): tagged sealed class with a multi-field named-field variant
/// (e.g. `Base64 { data: String, mediaType: String }`) must return
/// `ctx.readTreeAsValue(node, <Variant>::class.java)` directly — not wrap the
/// result in a second variant constructor call.
#[test]
fn tagged_deserializer_named_field_variant_no_double_wrap() {
    let en = make_enum(
        "InputDocument",
        Some("type"),
        false,
        Some("snake_case"),
        vec![
            make_variant("Url", Some("url"), vec![make_field("url", TypeRef::String)]),
            make_variant(
                "Base64",
                Some("base64"),
                vec![
                    make_field("data", TypeRef::String),
                    make_field("media_type", TypeRef::Named("MediaType".to_string())),
                ],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // Must return readTreeAsValue directly on payload (tag-stripped) — no `InputDocument.Base64(...)` wrap.
    // The explicit Kotlin type parameter avoids `Any!` inference.
    assert!(
        out.contains(
            "\"base64\" -> ctx.readTreeAsValue<InputDocument.Base64>(payload, InputDocument.Base64::class.java)"
        ),
        "tagged deserializer must return readTreeAsValue<T>(payload) directly for named-field variant; got:\n{out}",
    );
    assert!(
        !out.contains("InputDocument.Base64(ctx.readTreeAsValue"),
        "tagged deserializer must NOT wrap readTreeAsValue result in variant constructor; got:\n{out}",
    );
}

/// Regression (Bug A): tagged sealed class with a single-field newtype variant
/// (e.g. `Text(field0: String)` for `ContentPart`) must also return
/// `ctx.readTreeAsValue` directly without a variant constructor wrap.
#[test]
fn tagged_deserializer_newtype_variant_no_double_wrap() {
    let en = make_enum(
        "ContentPart",
        Some("type"),
        false,
        Some("snake_case"),
        vec![make_variant(
            "Text",
            Some("text"),
            vec![make_field("_0", TypeRef::Named("TextContent".to_string()))],
        )],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // Newtype variant: must wrap the inner-type result in the variant constructor.
    // The variant class (ContentPart.Text) is different from the inner type (TextContent).
    // Uses `payload` (tag-stripped node) — Bug D fix.
    assert!(
        out.contains("ContentPart.Text(ctx.readTreeAsValue<TextContent>(payload, TextContent::class.java))"),
        "tagged deserializer must wrap readTreeAsValue<InnerType>(payload) in variant constructor for newtype variant; got:\n{out}",
    );
}

/// Regression (Bug C): untagged sealed class with a `List<T>` variant where T
/// is a complex/named type must emit `constructCollectionType` with the correct
/// element class, not raw `List::class.java`.
#[test]
fn untagged_deserializer_list_of_named_type_uses_java_type() {
    let en = make_enum(
        "UserContent",
        None,
        true,
        None,
        vec![
            make_variant("Text", None, vec![make_field("_0", TypeRef::String)]),
            make_variant(
                "Parts",
                None,
                vec![make_field(
                    "_0",
                    TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                )],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    assert!(
        out.contains("ctx.typeFactory.constructCollectionType(List::class.java, ContentPart::class.java)"),
        "untagged deserializer must use constructCollectionType for List<ContentPart>; got:\n{out}",
    );
    assert!(
        !out.contains("ctx.readTreeAsValue(node, List::class.java)"),
        "untagged deserializer must NOT use raw List::class.java for List<ContentPart>; got:\n{out}",
    );
}

/// Unit-only enums (Kotlin `enum class`) must NOT get a `@JsonDeserialize`
/// annotation — they serialise to/from string values and Jackson handles
/// them natively.
#[test]
fn emit_enum_unit_only_does_not_emit_json_deserialize() {
    let en = make_enum(
        "FinishReason",
        None,
        false,
        None,
        vec![make_variant("Stop", None, vec![]), make_variant("Length", None, vec![])],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");
    assert!(
        !out.contains("@JsonDeserialize") && !out.contains("Deserializer"),
        "unit-only enum must not emit a deserializer; got:\n{out}",
    );
    assert!(
        out.contains("enum class FinishReason"),
        "must emit enum class; got:\n{out}"
    );
}

/// Regression (Bug D): tagged sealed-class deserializer must strip the tag field
/// from the JSON payload before passing it to readTreeAsValue.
///
/// Without this fix Jackson raises `UnrecognizedPropertyException` because the
/// inner type (e.g. `SystemMessage`) does not declare a `role` field.
#[test]
fn tagged_deserializer_strips_tag_field_from_payload() {
    let en = make_enum(
        "Message",
        Some("role"),
        false,
        None,
        vec![make_variant(
            "System",
            Some("system"),
            vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()))],
        )],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // The tag value must be read into a local variable first.
    assert!(
        out.contains("val tag = node.get(\"role\")?.asText()"),
        "deserializer must extract tag into a local variable; got:\n{out}",
    );
    // A tag-stripped payload must be created via deepCopy (cast) + remove.
    assert!(
            out.contains(
                "val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"role\") }"
            ),
            "deserializer must create tag-stripped payload via cast-safe deepCopy; got:\n{out}",
        );
    // The when-expression dispatches on `tag`, not `node.get(...)`.
    assert!(
        out.contains("return when (tag)"),
        "deserializer must dispatch on extracted tag variable; got:\n{out}",
    );
    // readTreeAsValue must receive `payload`, not the original `node`.
    assert!(
        out.contains("readTreeAsValue<SystemMessage>(payload, SystemMessage::class.java)"),
        "deserializer must pass tag-stripped payload to readTreeAsValue; got:\n{out}",
    );
    assert!(
        !out.contains("readTreeAsValue<SystemMessage>(node, SystemMessage::class.java)"),
        "deserializer must NOT pass un-stripped node to readTreeAsValue; got:\n{out}",
    );
}

/// Regression (Bug E): when a sealed-class variant field type has the same
/// simple name as a sibling variant, the field type must be fully-qualified
/// with the package path to prevent the Kotlin compiler from resolving the
/// type to the nested variant class (which causes self-recursion /
/// StackOverflowError in Jackson).
///
/// with neutral fixture package names.
/// Example: `ContentPart::ImageUrl { image_url: ImageUrl }` — inside
/// `ContentPart`, `ImageUrl` refers to the nested `data class ImageUrl` unless
/// the field type is explicitly qualified as `dev.sample_core.samplellm.android.ImageUrl`.
#[test]
fn sealed_class_variant_field_type_qualified_when_name_clashes_with_sibling_variant() {
    // Mirrors the real ContentPart::ImageUrl { image_url: ImageUrl } case.
    let en = make_enum(
        "ContentPart",
        Some("type"),
        false,
        None,
        vec![
            make_variant("Text", Some("text"), vec![make_field("text", TypeRef::String)]),
            make_variant(
                "ImageUrl",
                Some("image_url"),
                // Field type name `ImageUrl` matches variant name `ImageUrl` — clash!
                vec![make_field("image_url", TypeRef::Named("ImageUrl".to_string()))],
            ),
        ],
    );
    let mut out = String::new();
    // Provide a non-empty package so disambiguation can emit the FQN.
    emit_enum(&en, &mut out, "dev.sample_crate.samplellm.android");

    // The variant data class must qualify the field type to avoid self-reference.
    assert!(
        out.contains("val imageUrl: dev.sample_crate.samplellm.android.ImageUrl"),
        "variant field type must be package-qualified when it clashes with a sibling variant name; got:\n{out}",
    );
    // The variant data class header itself is unqualified.
    assert!(
        out.contains("data class ImageUrl("),
        "variant class declaration must still use simple name; got:\n{out}",
    );
}

/// Non-clashing variant field types must NOT be package-qualified (verbosity
/// guard — only disambiguate when the field type name matches a sibling variant).
#[test]
fn sealed_class_variant_field_type_unqualified_when_no_clash() {
    let en = make_enum(
        "ContentPart",
        Some("type"),
        false,
        None,
        vec![make_variant(
            "Document",
            Some("document"),
            // `DocumentContent` does not match any variant name — no qualification needed.
            vec![make_field("document", TypeRef::Named("DocumentContent".to_string()))],
        )],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "dev.sample_crate.samplellm.android");

    // Must use the simple name.
    assert!(
        out.contains("val document: DocumentContent"),
        "non-clashing field type must remain unqualified; got:\n{out}",
    );
    // Must NOT spuriously qualify.
    assert!(
        !out.contains("dev.sample_crate.samplellm.android.DocumentContent"),
        "non-clashing field type must not be package-qualified; got:\n{out}",
    );
}

/// Regression (Bug G): sealed class variant data classes must each carry a bare
/// `@JsonDeserialize` annotation to prevent Jackson annotation inheritance from
/// the parent sealed class.
///
/// When the sealed class has `@JsonDeserialize(using = FooDeserializer::class)`,
/// every nested variant class inherits that annotation.  This causes infinite
/// recursion (or an "Unknown tag" error on the stripped payload) when
/// `ctx.readTreeAsValue(payload, Foo.Variant::class.java)` is called inside
/// `FooDeserializer` — Jackson re-invokes `FooDeserializer` for the variant
/// class, which then fails because the variant's payload has no tag field.
///
/// Emitting bare `@JsonDeserialize` (defaulting to `using = JsonDeserializer.None`)
/// on each variant data class overrides the inherited annotation with the default
/// POJO deserializer, breaking the recursion cycle.
#[test]
fn sealed_class_variant_data_classes_get_json_deserialize_reset_annotation() {
    let en = make_enum(
        "InputDocument",
        Some("type"),
        false,
        Some("snake_case"),
        vec![
            make_variant("Url", Some("document_url"), vec![make_field("url", TypeRef::String)]),
            make_variant(
                "Base64",
                Some("base64"),
                vec![
                    make_field("data", TypeRef::String),
                    make_field("mediaType", TypeRef::String),
                ],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // Named-field struct variants must carry @JsonDeserialize(using = None) and
    // @JsonSerialize(using = None) to reset the inherited custom (de)serializers.
    // The parent tagged deserializer calls readTreeAsValue(payload, Variant::class.java)
    // which would loop back into the parent deserializer without this reset.
    assert!(
            out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n    data class Url("),
            "Url variant must have @JsonDeserialize(using=None) and @JsonSerialize(using=None) reset annotations; got:\n{out}",
        );
    assert!(
            out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n    data class Base64("),
            "Base64 variant must have @JsonDeserialize(using=None) and @JsonSerialize(using=None) reset annotations; got:\n{out}",
        );
}

/// Regression (Bug G — untagged): newtype variants of untagged sealed classes do NOT
/// need reset annotations because the parent serializer dispatches via the inner value
/// type (no recursion). However, the serializer for Vec<SealedClass> variants must use
/// provider.findValueSerializer so the sealed-class serializer (which adds "type") is
/// invoked per element, not the variant's reset-to-None subtype serializer.
#[test]
fn untagged_sealed_class_vec_variant_serializer_uses_declared_type_serializer() {
    let en = make_enum(
        "UserContent",
        None,
        true,
        None,
        vec![
            make_variant("Text", None, vec![make_field("_0", TypeRef::String)]),
            make_variant(
                "Parts",
                None,
                vec![make_field(
                    "_0",
                    TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                )],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // Newtype variants are NOT given reset annotations (no recursion risk).
    assert!(
            !out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize\n    data class Text("),
            "Text newtype variant must NOT have reset annotations; got:\n{out}",
        );
    // Parts serializer must use provider.findValueSerializer for element dispatch.
    assert!(
        out.contains("provider.findValueSerializer(ContentPart::class.java)"),
        "Parts serializer must use provider.findValueSerializer(ContentPart::class.java); got:\n{out}",
    );
    // Text serializer uses direct mapper.writeValue (String is not a sealed class).
    assert!(
        out.contains("is UserContent.Text -> mapper.writeValue(gen, value.value)"),
        "Text serializer must use mapper.writeValue; got:\n{out}",
    );
}

/// Regression: untagged sealed-class serializer must use the payload-derived field
/// name (e.g. `value`) rather than the literal `field0`.  Without this fix the
/// generated `when`-branch emits `value.field0` which is an unresolved reference
/// because the data-class declaration uses the name derived by
/// `kotlin_field_name_with_type` (e.g. `Single(val value: String)`).
#[test]
fn untagged_serializer_tuple_variant_uses_payload_derived_field_name() {
    // EmbeddingInput pattern: single-field tuple variants whose field type is a
    // primitive (String, List<String>) → field name must be `value`, not `field0`.
    let en = make_enum(
        "EmbeddingInput",
        None,
        true,
        None,
        vec![
            make_variant("Single", None, vec![make_field("_0", TypeRef::String)]),
            make_variant(
                "Multiple",
                None,
                vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
            ),
        ],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // Serializer when-branches must reference `value.value`, not `value.field0`.
    assert!(
        out.contains("-> mapper.writeValue(gen, value.value)"),
        "untagged serializer must use payload-derived field name `value`; got:\n{out}",
    );
    assert!(
        !out.contains("value.field0"),
        "untagged serializer must NOT use hardcoded `field0`; got:\n{out}",
    );
}

/// Regression (Bug G — serializer cast): named-field struct variants in a tagged
/// sealed class serializer must cast `value` to the concrete variant type before
/// calling `mapper.valueToTree`.  Without the cast, Jackson would resolve the
/// serializer against the parent sealed class type, re-triggering the custom
/// serializer and causing infinite recursion.
#[test]
fn tagged_serializer_named_field_variant_casts_to_concrete_type() {
    let en = make_enum(
        "InputDocument",
        Some("type"),
        false,
        Some("snake_case"),
        vec![make_variant(
            "Url",
            Some("document_url"),
            vec![make_field("url", TypeRef::String)],
        )],
    );
    let mut out = String::new();
    emit_enum(&en, &mut out, "");

    // The serializer must cast `value` to `InputDocument.Url` before calling
    // valueToTree so Jackson uses the variant class's serializer (reset to
    // default POJO), not the parent sealed class's custom serializer.
    assert!(
            out.contains("mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value as InputDocument.Url) as com.fasterxml.jackson.databind.node.ObjectNode"),
            "tagged serializer must cast value to concrete variant type; got:\n{out}",
        );
    // Must NOT call valueToTree on `value` without a cast.
    assert!(
        !out.contains("mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value) as"),
        "tagged serializer must NOT call valueToTree on un-cast parent-type value; got:\n{out}",
    );
}
