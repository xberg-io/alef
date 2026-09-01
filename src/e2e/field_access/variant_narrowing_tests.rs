//! What the C# and Dart accessor renderers actually emit for a path that steps into a
//! tagged-union variant.
//!
//! ~keep The map-building tests next to each language's codegen prove the map is right; these
//! prove the renderers consume it, which is a different claim and the one that was false. A
//! correct map that no renderer reads produces exactly the broken output this change exists to
//! fix, and unit tests on the map alone would have passed the whole time. Fixture names are
//! neutral per `project-agnostic-codegen`.

use super::{FieldResolver, VariantAccessorMap};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

const ROOT: &str = "SampleResult";
const UNION: &str = "SamplePayload";

fn field(name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        ..FieldDef::default()
    }
}

/// `SampleResult { detail: SamplePayload }`, `SamplePayload::Alpha(AlphaDetails)`,
/// `AlphaDetails { label: String }` — the minimal shape of a path that crosses a variant.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: ROOT.to_string(),
            fields: vec![field("detail", UNION)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "AlphaDetails".to_string(),
            fields: vec![field("label", "String")],
            ..TypeDef::default()
        },
    ]
}

fn enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: UNION.to_string(),
        variants: vec![EnumVariant {
            name: "Alpha".to_string(),
            fields: vec![field("_0", "AlphaDetails")],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }]
}

fn resolver_with(accessors: VariantAccessorMap) -> FieldResolver {
    FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs(), &enums()),
        Some(ROOT.to_string()),
    )
    .with_variant_accessors(accessors)
}

fn csharp_accessors() -> VariantAccessorMap {
    crate::e2e::codegen::csharp::build_variant_accessor_map_for_tests(&enums())
}

fn dart_accessors() -> VariantAccessorMap {
    crate::e2e::codegen::dart::build_variant_accessor_map_for_tests(&enums())
}

#[test]
fn csharp_should_narrow_through_the_generated_accessor_not_the_variant_type() {
    let rendered = resolver_with(csharp_accessors()).accessor("detail.alpha.label", "csharp", "result");

    assert_eq!(
        rendered, "result.Detail.AsAlpha!.Label",
        "naming the variant type (`.Alpha`) is CS0572; the generated `As<Variant>` property is \
         the only expression form that compiles"
    );
}

#[test]
fn dart_should_cast_to_the_variant_subclass_and_read_its_payload() {
    let rendered = resolver_with(dart_accessors()).accessor("detail.alpha.label", "dart", "result");

    assert_eq!(
        rendered, "(result.detail as SamplePayload_Alpha).field0.label",
        "a freezed sealed class exposes no per-variant getter, so the narrowing has to be a cast"
    );
}

/// The guard against the failure mode this whole change is about: an unfurnished resolver must
/// keep rendering exactly as it did before, so no other language's output can move.
#[test]
fn an_empty_map_should_render_exactly_as_before() {
    let bare = resolver_with(VariantAccessorMap::default());

    assert_eq!(
        bare.accessor("detail.alpha.label", "csharp", "result"),
        "result.Detail.Alpha.Label"
    );
    assert_eq!(
        bare.accessor("detail.alpha.label", "dart", "result"),
        "result.detail.alpha.label"
    );
}

/// A path that never crosses a variant must be untouched even with the map populated.
#[test]
fn a_path_that_crosses_no_variant_is_unaffected_by_a_populated_map() {
    let resolver = resolver_with(csharp_accessors());

    assert_eq!(resolver.accessor("detail", "csharp", "result"), "result.Detail");
}

/// The wildcard (`container[].field`) form must narrow identically to the result-anchored one.
///
/// ~keep Today this holds by construction rather than by care: `accessor` and
/// `element_accessor` both delegate to `render_relative_to`, so there is only one place the
/// narrowing can be applied and no second path for it to be missing from. That is exactly the
/// property that did NOT hold when a napi enum fix landed on the non-wildcard path while
/// `render_wildcard_assertion` returned earlier, and four green tests reported success. This
/// pins the single-path property so that reintroducing a separate wildcard renderer fails here
/// rather than shipping a half-applied fix.
#[test]
fn the_wildcard_element_form_narrows_the_same_way_as_the_result_anchored_form() {
    let resolver = resolver_with(csharp_accessors());

    assert_eq!(
        resolver.element_accessor("detail.alpha.label", "csharp", "item"),
        "item.Detail.AsAlpha!.Label",
        "an element-anchored path crosses the same variant and must narrow the same way"
    );
    assert_eq!(
        resolver_with(dart_accessors()).element_accessor("detail.alpha.label", "dart", "item"),
        "(item.detail as SamplePayload_Alpha).field0.label"
    );
}
