use super::mirror_conversions::emit_from_mirror_to_core_struct;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};

fn field(name: &str, binding_excluded: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional: false,
        binding_excluded,
        ..Default::default()
    }
}

fn typ(name: &str, has_default: bool, has_stripped_cfg_fields: bool, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("source::{name}"),
        fields,
        is_clone: true,
        has_default,
        has_stripped_cfg_fields,
        ..Default::default()
    }
}

#[test]
fn mirror_to_core_binding_excluded_with_default_uses_spread() {
    let ty = typ(
        "DefaultedWithExcluded",
        true,
        true,
        vec![field("name", false), field("internal", true)],
    );
    let mut out = String::new();
    emit_from_mirror_to_core_struct(&mut out, &ty, "source");

    assert!(
        out.contains("..Default::default()"),
        "spread should be emitted when has_default && has_stripped_cfg_fields; got:\n{out}"
    );
    assert!(
        out.contains("#[allow(clippy::needless_update)]"),
        "needless_update allow should accompany the emitted spread; got:\n{out}"
    );
    assert!(
        !out.contains("internal: Default::default()"),
        "binding-excluded field should be skipped when has_default is true; got:\n{out}"
    );
}

#[test]
fn mirror_to_core_stripped_cfg_without_default_omits_spread() {
    let ty = typ("NoDefaultStripped", false, true, vec![field("name", false)]);
    let mut out = String::new();
    emit_from_mirror_to_core_struct(&mut out, &ty, "source");

    assert!(
        !out.contains("..Default::default()"),
        "spread must NOT be emitted when has_default is false; got:\n{out}"
    );
    assert!(
        !out.contains("#[allow(clippy::needless_update)]"),
        "needless_update allow must NOT be emitted when no spread; got:\n{out}"
    );
}

#[test]
fn mirror_to_core_binding_excluded_without_default_emits_explicit_only() {
    let ty = typ(
        "NoDefaultExcluded",
        false,
        false,
        vec![field("name", false), field("internal", true)],
    );
    let mut out = String::new();
    emit_from_mirror_to_core_struct(&mut out, &ty, "source");

    assert!(
        !out.contains("..Default::default()"),
        "spread must NOT be emitted when has_default is false; got:\n{out}"
    );
    assert!(
        out.contains("internal: Default::default()"),
        "binding-excluded field must be explicitly defaulted; got:\n{out}"
    );
}

#[test]
fn mirror_to_core_fully_mirrored_with_default_emits_spread() {
    // Forward-compatibility: a has_default core type with every field mirrored
    // must still get the spread trailer, so an additive core field falls back
    // to its default instead of failing E0063 until the bindings are regenerated.
    let ty = typ("Plain", true, false, vec![field("name", false), field("value", false)]);
    let mut out = String::new();
    emit_from_mirror_to_core_struct(&mut out, &ty, "source");

    assert!(
        out.contains("..Default::default()"),
        "has_default core type must always get the spread trailer; got:\n{out}"
    );
    assert!(
        out.contains("#[allow(clippy::needless_update)]"),
        "needless_update allow should accompany the emitted spread; got:\n{out}"
    );
}
