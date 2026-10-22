use super::*;

/// A single-field tuple variant carrying its own `#[serde(untagged)]` (e.g.
/// `OutputFormat::Custom(String)`, `crates/xberg/src/core/config/formats.rs`) serializes as its
/// bare field value, with no discriminant on the wire at all. The generic getter parses a tag out
/// of the whole enum's JSON and so can never find it -- this variant must instead be matched
/// directly on `&self.inner`, the same way the typed `Named`-payload accessor is.
#[test]
fn untagged_variant_getter_matches_the_rust_variant_instead_of_a_json_tag() {
    let mut custom = variant("Custom", vec![field("_0")]);
    custom.is_tuple = true;
    custom.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&enum_def("OutputFormat", vec![custom]), "core");

    assert!(
        generated.contains("crate::OutputFormat::Custom(data) => {"),
        "must match the variant directly rather than reading a tag out of the JSON: {generated}"
    );
    assert!(
        generated.contains("serde_json::to_value(data)"),
        "must serialize only the matched field, not the whole enum: {generated}"
    );
    assert!(
        !generated.contains("\"custom\""),
        "an untagged variant has no wire tag to look up: {generated}"
    );
}

/// The untagged accessor drops a foreign-cfg-gated variant's match arm the same way the typed
/// accessor does, falling back to the `_ => Ok(None)` catch-all rather than referencing
/// `core_path::Variant` under a `#[cfg(...)]` this crate cannot declare as a Cargo feature.
#[test]
fn untagged_variant_getter_drops_foreign_cfg_gated_arm() {
    let mut custom = variant("Custom", vec![field("_0")]);
    custom.is_tuple = true;
    custom.serde_untagged = true;
    custom.cfg = Some(r#"feature = "extra""#.to_string());
    let generated = gen_pyo3_data_enum(&enum_def("OutputFormat", vec![custom]), "core");

    assert!(
        !generated.contains("crate::OutputFormat::Custom(data)"),
        "a foreign cfg this crate cannot declare must drop the match arm entirely: {generated}"
    );
    assert!(generated.contains("_ => Ok(None),"), "{generated}");
}

/// A whole enum marked `#[serde(untagged)]` serializes to the bare payload, so no variant has a key
/// to look up. Each getter must match the Rust variant held by `&self.inner`: the enum already
/// knows which variant it holds, so there is no shape to guess.
#[test]
fn untagged_enum_getters_match_the_rust_variant_instead_of_a_json_key() {
    let mut text = variant("Text", vec![field("_0")]);
    text.is_tuple = true;
    let mut parts = variant("Parts", vec![field("_0")]);
    parts.is_tuple = true;
    parts.fields[0].ty = TypeRef::Vec(Box::new(TypeRef::String));
    let mut def = enum_def("AssistantContent", vec![text, parts]);
    def.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&def, "core");

    assert!(
        generated.contains("crate::AssistantContent::Text(data) => {"),
        "{generated}"
    );
    assert!(
        generated.contains("crate::AssistantContent::Parts(data) => {"),
        "{generated}"
    );
    assert!(
        !generated.contains("values.get(\"Text\")") && !generated.contains("values.get(\"Parts\")"),
        "an untagged value never has a key named after its variant: {generated}"
    );
}

/// Two variants with the same JSON shape cannot be told apart by their value, so the getters
/// follow the Rust variant: each getter has its own match arm and the other variant falls to
/// `Ok(None)`. A value holding `Second` never answers through `first`.
#[test]
fn untagged_enum_variants_with_the_same_json_shape_each_match_only_their_own_variant() {
    let mut first = variant("First", vec![field("_0")]);
    first.is_tuple = true;
    let mut second = variant("Second", vec![field("_0")]);
    second.is_tuple = true;
    let mut def = enum_def("Either", vec![first, second]);
    def.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&def, "core");

    let first_body = generated_method_body(&generated, "first").replace(' ', "");
    let second_body = generated_method_body(&generated, "second").replace(' ', "");
    assert!(first_body.contains("crate::Either::First(data)"), "{first_body}");
    assert!(!first_body.contains("crate::Either::Second"), "{first_body}");
    assert!(second_body.contains("crate::Either::Second(data)"), "{second_body}");
    assert!(!second_body.contains("crate::Either::First"), "{second_body}");
    assert!(first_body.contains("_=>Ok(None)"), "{first_body}");
}

/// Struct and multi-field variants of an untagged enum serialize as an object or an array with no
/// key either: the getter matches the variant and serializes the whole enum, which is the payload.
#[test]
fn untagged_enum_struct_variant_getter_serializes_the_whole_value() {
    let mut named = variant("Named", vec![field("id"), field("label")]);
    named.is_tuple = false;
    let mut def = enum_def("Shape", vec![named]);
    def.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&def, "core");

    let body = generated_method_body(&generated, "named_payload").replace(' ', "");
    assert!(body.contains("crate::Shape::Named{..}=>{"), "{body}");
    assert!(body.contains("serde_json::to_value(&self.inner)"), "{body}");
    assert!(!body.contains("values.get("), "{body}");
}

/// A unit variant and a multi-field tuple variant of an untagged enum have no single field to bind:
/// the getter matches the bare variant name or `(..)` and serializes the whole value.
#[test]
fn untagged_enum_unit_and_multi_field_variants_serialize_the_whole_value() {
    for (name, field_count, arm) in [
        ("Empty", 0, "crate::Mixed::Empty=>{"),
        ("Pair", 2, "crate::Mixed::Pair(..)=>{"),
    ] {
        let fields = (0..field_count).map(|i| field(&format!("_{i}"))).collect();
        let mut held = variant(name, fields);
        held.is_tuple = true;
        let mut def = enum_def("Mixed", vec![held]);
        def.serde_untagged = true;
        let generated = gen_pyo3_data_enum(&def, "core");

        let body = generated_method_body(&generated, &name.to_lowercase()).replace(' ', "");
        assert!(body.contains(arm), "{name}: {body}");
        assert!(body.contains("serde_json::to_value(&self.inner)"), "{name}: {body}");
    }
}

/// A struct variant with one field is not a tuple variant: `Core::Object(data)` does not compile
/// for it, so the getter matches `Object { .. }` and serializes the whole value.
#[test]
fn untagged_enum_single_field_struct_variant_getter_matches_the_struct_pattern() {
    let mut object = variant("Object", vec![field("text")]);
    object.is_tuple = false;
    let mut def = enum_def("Document", vec![object]);
    def.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&def, "core");

    let body = generated_method_body(&generated, "object_payload").replace(' ', "");
    assert!(body.contains("crate::Document::Object{..}=>{"), "{body}");
    assert!(!body.contains("Object(data)"), "{body}");
    assert!(body.contains("serde_json::to_value(&self.inner)"), "{body}");
}

#[test]
fn per_variant_untagged_single_field_struct_getter_matches_the_struct_pattern() {
    let mut object = variant("Object", vec![field("text")]);
    object.serde_untagged = true;
    let generated = gen_pyo3_data_enum(&enum_def("Document", vec![object]), "core");

    let body = generated_method_body(&generated, "object_payload").replace(' ', "");
    assert!(body.contains("crate::Document::Object{..}=>{"), "{body}");
    assert!(!body.contains("Object(data)"), "{body}");
    assert!(body.contains("serde_json::to_value(&self.inner)"), "{body}");
    assert!(!body.contains("values.get("), "{body}");
}
