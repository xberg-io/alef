//! Regression coverage for the Java sealed-union nested-record shadowing defect.
//!
//! JLS §6.4.1: a member type declared inside a class/interface body (here, the
//! `record <Variant>(...)` nested inside `sealed interface X`) shadows any
//! same-named top-level type for the rest of that body. When a Rust enum variant's
//! name equals its payload type's name — e.g. `ContentPart::ImageUrl { image_url:
//! ImageUrl }`, where the inner `ImageUrl` is a sibling top-level struct — the naive
//! emission `record ImageUrl(@JsonProperty("image_url") ImageUrl imageUrl)` makes
//! the field type resolve to the record itself: construction becomes impossible and
//! deserialization silently drops the payload. See
//! `crates::backends::java::gen_bindings::helpers::qualify_shadowed_type`.

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
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "io.xberg.literllm"
"#,
    )
    .expect("valid Java config");
    config.resolve().expect("resolved Java config").remove(0)
}

/// Builds a `ContentPart`-shaped enum: a tagged union whose `ImageUrl` variant
/// carries a single named field of type `ImageUrl` — the sibling top-level struct,
/// not the variant itself.
fn content_part_enum() -> EnumDef {
    EnumDef {
        name: "ContentPart".to_string(),
        rust_path: "demo::ContentPart".to_string(),
        serde_tag: Some("type".to_string()),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: "Plain text.".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "ImageUrl".to_string(),
                fields: vec![FieldDef {
                    name: "image_url".to_string(),
                    ty: TypeRef::Named("ImageUrl".to_string()),
                    ..Default::default()
                }],
                doc: "Image identified by URL.".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn enum_variant_name_matching_payload_type_emits_fully_qualified_component() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![content_part_enum()],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let result = backend.generate_bindings(&api, &test_config());
    assert!(result.is_ok(), "generation failed: {:?}", result.err());
    let files = result.unwrap();

    let content_part_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("ContentPart.java"))
        .expect("ContentPart.java not generated");
    let content = &content_part_file.content;

    assert!(
        content.contains("record ImageUrl(@JsonProperty(\"image_url\") io.xberg.literllm.ImageUrl imageUrl)"),
        "ImageUrl variant's payload field must be fully qualified to avoid the nested \
         record shadowing the sibling top-level ImageUrl type. Got:\n{content}"
    );
    assert!(
        !content.contains("record ImageUrl(@JsonProperty(\"image_url\") ImageUrl imageUrl)"),
        "the bare (unqualified) form is self-referential and must not be emitted. Got:\n{content}"
    );

    // Text is unaffected — its field type (String) never collides with a variant name.
    assert!(
        content.contains("record Text(@JsonProperty(\"text\") String text)"),
        "non-colliding variant fields must remain unqualified. Got:\n{content}"
    );
}
