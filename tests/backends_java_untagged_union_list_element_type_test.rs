//! Regression coverage for the Java untagged-union wrapper losing a polymorphic element's
//! serde discriminator.
//!
//! The wrapper's only general-purpose factory is `ofObject(Object)`, which used to be a bare
//! `MAPPER.valueToTree(o)`. Java erases a generic collection's element type, so Jackson builds a
//! `CollectionSerializer` with no element `TypeSerializer` and resolves each element through the
//! *untyped* `findValueSerializer` lookup — the `@JsonTypeInfo` discriminator of a polymorphic
//! element type is silently dropped. The same value handed over on its own keeps its
//! discriminator, because a root value goes through `findTypedValueSerializer`; only the erased
//! container path is affected. The tag-less tree is then frozen into the wrapper's stored
//! `JsonNode`, so nothing downstream can repair it and the Rust core rejects the payload as not
//! matching any variant of the untagged enum.
//!
//! The fix pins the element type with the same `writerFor(constructCollectionType(..))` mechanism
//! the DTO marshal path already uses (`gen_bindings::marshal::build_collection_writer_for`). ~keep

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

fn test_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "dev.sample"
"#,
    )
    .expect("valid Java config");
    config.resolve().expect("resolved Java config").remove(0)
}

fn newtype_variant(name: &str, ty: TypeRef) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "0".to_string(),
            ty,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// `#[serde(tag = "kind")]` — the polymorphic element type whose discriminator is at stake.
fn media_part_enum() -> EnumDef {
    EnumDef {
        name: "MediaPart".to_string(),
        rust_path: "sample_crate::MediaPart".to_string(),
        serde_tag: Some("kind".to_string()),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Caption".to_string(),
                fields: vec![FieldDef {
                    name: "caption".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Frame".to_string(),
                fields: vec![FieldDef {
                    name: "frame".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn untagged_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{name}"),
        serde_untagged: true,
        has_serde: true,
        variants,
        ..Default::default()
    }
}

fn generate(enums: Vec<EnumDef>) -> Vec<alef::core::backend::GeneratedFile> {
    let api = ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        enums,
        ..Default::default()
    };
    JavaBackend
        .generate_bindings(&api, &test_config())
        .expect("Java generation must succeed")
}

fn wrapper_source(files: &[alef::core::backend::GeneratedFile], class: &str) -> String {
    let needle = format!("{class}.java");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with(&needle))
        .unwrap_or_else(|| panic!("{needle} not generated"))
        .content
        .clone()
}

/// The defect: a `Vec<PolymorphicType>` variant must reach Jackson through a writer whose element
/// type is pinned, never through the erasing `valueToTree` path.
#[test]
fn ofobject_pins_the_element_type_of_a_named_list_variant() {
    let files = generate(vec![
        media_part_enum(),
        untagged_enum(
            "MediaContent",
            vec![
                newtype_variant("Text", TypeRef::String),
                newtype_variant("Parts", TypeRef::Vec(Box::new(TypeRef::Named("MediaPart".to_string())))),
            ],
        ),
    ]);
    let source = wrapper_source(&files, "MediaContent");

    assert!(
        source.contains(
            "private static final ObjectWriter LIST_WRITER_MEDIA_PART =\n        \
             MAPPER.writerFor(MAPPER.getTypeFactory()\
             .constructCollectionType(java.util.List.class, MediaPart.class));"
        ),
        "the wrapper must declare a writer with the list element type pinned to MediaPart. \
         Got:\n{source}"
    );
    assert!(
        source.contains("import com.fasterxml.jackson.databind.ObjectWriter;"),
        "the pinned writer needs its import. Got:\n{source}"
    );
    assert!(
        source.contains("if (first instanceof MediaPart) {")
            && source.contains("return new MediaContent(typedTree(LIST_WRITER_MEDIA_PART, o));"),
        "ofObject must route a List of MediaPart through the pinned writer. Got:\n{source}"
    );
    assert!(
        source.contains("if (o instanceof List<?> items && !items.isEmpty()) {"),
        "the pinned-writer branch must be guarded on the argument actually being a non-empty list. \
         Got:\n{source}"
    );

    assert!(
        source.contains("return new MediaContent(MAPPER.valueToTree(o));"),
        "a non-list argument must keep the plain conversion, which already carries the \
         discriminator. Got:\n{source}"
    );
}

/// The pin is per element type, not "is a list": an untagged union may carry both a `Vec<String>`
/// and a `Vec<PolymorphicType>` variant, and a `List<String>` argument must keep reaching
/// `valueToTree` — a `String` can never carry a discriminator, and routing it through the
/// `MediaPart` writer would make Jackson throw on a previously working call.
#[test]
fn ofobject_pins_only_the_named_element_type_when_a_string_list_variant_coexists() {
    let files = generate(vec![
        media_part_enum(),
        untagged_enum(
            "BatchInput",
            vec![
                newtype_variant("Single", TypeRef::String),
                newtype_variant("Multiple", TypeRef::Vec(Box::new(TypeRef::String))),
                newtype_variant("Parts", TypeRef::Vec(Box::new(TypeRef::Named("MediaPart".to_string())))),
            ],
        ),
    ]);
    let source = wrapper_source(&files, "BatchInput");

    assert_eq!(
        source.matches("private static final ObjectWriter ").count(),
        1,
        "exactly one pinned writer — the String element type must not get one. Got:\n{source}"
    );
    assert!(
        source.contains("LIST_WRITER_MEDIA_PART"),
        "the MediaPart element type must be the one that is pinned. Got:\n{source}"
    );
    assert!(
        !source.contains("String.class));"),
        "a String element type must not be pinned; it can never carry a discriminator. \
         Got:\n{source}"
    );
    assert_eq!(
        source.matches("if (first instanceof ").count(),
        1,
        "one element-type guard, matching the one pinned writer. Got:\n{source}"
    );
}

/// A union with no collection variant must be byte-for-byte unchanged: no writer constant, no
/// extra imports, no unreachable helper.
#[test]
fn ofobject_keeps_the_plain_conversion_when_no_variant_is_a_list() {
    let files = generate(vec![untagged_enum(
        "ScalarChoice",
        vec![
            newtype_variant("Text", TypeRef::String),
            newtype_variant("Count", TypeRef::Primitive(alef::core::ir::PrimitiveType::U32)),
        ],
    )]);
    let source = wrapper_source(&files, "ScalarChoice");

    assert!(
        source.contains(
            "    public static ScalarChoice ofObject(final Object o) {\n        \
             return new ScalarChoice(MAPPER.valueToTree(o));\n    }"
        ),
        "with no list variant, ofObject must stay the plain one-liner. Got:\n{source}"
    );
    assert!(
        !source.contains("ObjectWriter"),
        "no pinned writer, and therefore no ObjectWriter import. Got:\n{source}"
    );
    assert!(
        !source.contains("JsonProcessingException"),
        "no typedTree helper, and therefore no JsonProcessingException import. Got:\n{source}"
    );
}
