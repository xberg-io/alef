use super::mirror_conversions::{emit_from_impl_for_struct, emit_from_mirror_to_core_struct};
use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeWrapper, TypeDef, TypeRef};

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

/// The regression this task fixes: a struct wholly gated behind a Cargo feature (e.g. a type
/// nested in a `#[cfg(feature = "thumbnails")]` module) carries that gate on `TypeDef::cfg`.
/// `emit_from_impl_for_struct`'s `impl From<core_ty> for Mirror` names `core_ty` -- the host
/// path -- directly, so leaving the impl unconditional emits a reference to a path that does
/// not exist in a build excluding that feature (E0433 `cannot find` in the real failure this
/// mirrors). Before the fix, `source_cfg` was passed to the template but the template never
/// used it, so this impl was always emitted with no `#[cfg(...)]` at all.
#[test]
fn from_core_impl_carries_whole_type_cfg_gate() {
    let ty = TypeDef {
        name: "OcrResult".to_string(),
        rust_path: "sample_lib::thumbnails::OcrResult".to_string(),
        cfg: Some(r#"feature = "thumbnails""#.to_string()),
        fields: vec![field("text", false)],
        ..Default::default()
    };
    let mut out = String::new();
    emit_from_impl_for_struct(&mut out, &ty, "sample_lib");

    assert!(
        out.contains("sample_lib::thumbnails::OcrResult"),
        "expected the gated host path to still be referenced in the impl, got:\n{out}"
    );
    assert_eq!(
        out.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the whole-type gate must land on the From<core_ty> impl exactly once, got:\n{out}"
    );
}

/// Same regression, the mirror-to-core direction: `emit_from_mirror_to_core_struct`'s
/// `impl From<Mirror> for core_ty` also names the host path directly and must carry the same
/// whole-type gate.
#[test]
fn from_mirror_impl_carries_whole_type_cfg_gate() {
    let ty = TypeDef {
        name: "OcrResult".to_string(),
        rust_path: "sample_lib::thumbnails::OcrResult".to_string(),
        cfg: Some(r#"feature = "thumbnails""#.to_string()),
        fields: vec![field("text", false)],
        ..Default::default()
    };
    let mut out = String::new();
    emit_from_mirror_to_core_struct(&mut out, &ty, "sample_lib");

    assert_eq!(
        out.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the whole-type gate must land on the From<Mirror> impl exactly once, got:\n{out}"
    );
}

/// Negative control: an ungated type (`TypeDef::cfg` is `None`) must emit no `#[cfg(...)]` at
/// all on either impl -- the fix must not gate unconditionally.
#[test]
fn ungated_type_emits_no_cfg_on_either_impl() {
    let ty = typ("PlainResult", false, false, vec![field("text", false)]);

    let mut out_core = String::new();
    emit_from_impl_for_struct(&mut out_core, &ty, "sample_lib");
    assert!(
        !out_core.contains("#[cfg("),
        "ungated type must not emit #[cfg(...)] in From<core_ty> impl, got:\n{out_core}"
    );

    let mut out_mirror = String::new();
    emit_from_mirror_to_core_struct(&mut out_mirror, &ty, "sample_lib");
    assert!(
        !out_mirror.contains("#[cfg("),
        "ungated type must not emit #[cfg(...)] in From<Mirror> impl, got:\n{out_mirror}"
    );
}

#[test]
fn transparent_string_wrappers_use_explicit_operations_in_both_directions() {
    let wrapper = |containers| {
        Some(NewtypeWrapper::transparent_string(
            "source::SecretString",
            "from",
            "into_inner",
            containers,
        ))
    };
    let ty = typ(
        "Credentials",
        false,
        false,
        vec![
            FieldDef {
                name: "token".to_string(),
                ty: TypeRef::String,
                newtype_wrapper: wrapper(vec![]),
                ..Default::default()
            },
            FieldDef {
                name: "optional".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                newtype_wrapper: wrapper(vec![NewtypeContainer::Optional]),
                ..Default::default()
            },
            FieldDef {
                name: "headers".to_string(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                newtype_wrapper: wrapper(vec![NewtypeContainer::MapValue]),
                ..Default::default()
            },
        ],
    );

    let mut from_core = String::new();
    emit_from_impl_for_struct(&mut from_core, &ty, "source");
    assert!(from_core.contains("token: (v.token).into_inner()"), "got:\n{from_core}");
    assert!(
        from_core.contains("(v.optional).map(|value| (value).into_inner())"),
        "got:\n{from_core}"
    );
    assert!(
        from_core.contains("map(|(key, value)| (key, (value).into_inner()))"),
        "got:\n{from_core}"
    );
    assert!(!from_core.contains("to_string()"), "got:\n{from_core}");

    let mut to_core = String::new();
    emit_from_mirror_to_core_struct(&mut to_core, &ty, "source");
    assert!(
        to_core.contains("token: source::SecretString::from(v.token)"),
        "got:\n{to_core}"
    );
    assert!(
        to_core.contains("map(|value| source::SecretString::from(value))"),
        "got:\n{to_core}"
    );
    assert!(
        to_core.contains("map(|(key, value)| (key, source::SecretString::from(value)))"),
        "got:\n{to_core}"
    );
}
