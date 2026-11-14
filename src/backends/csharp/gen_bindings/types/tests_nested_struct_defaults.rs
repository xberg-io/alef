//! Coverage for the container-level (no per-field `#[serde(default)]`) nested-struct-default
//! case: a field whose type is another emitted record, defaulted only through the *container's*
//! `impl Default` (so `field.default` is `None`, and the only signal is
//! `typed_default == Some(DefaultValue::Empty)`).
//!
//! `tests.rs`'s `record_type_nested_record_field_is_constructed_not_null` already covers this
//! shape for a field carrying its own `#[serde(default)]` attribute (`field.default.is_some()`).
//! These tests cover the same construction/fallback behaviour when that per-field attribute is
//! absent and `field.default` is `None` — the extractor's postprocess pass narrows enum-typed
//! `Empty` fields to `EnumVariant` (already renderable), but a *struct*-typed field whose only
//! own default is `Default::default()` still resolves through `Empty`, and it is this
//! renderer's job to recognize the nested record is itself fully default-constructible instead
//! of falling to `required`.

use super::gen_record_type;
use super::tests::{field, named_record_type, record_type};
use crate::core::ir::{DefaultValue, TypeRef};
use std::collections::HashSet;

fn render_with_types(typ: &crate::core::ir::TypeDef, types: &[crate::core::ir::TypeDef]) -> String {
    gen_record_type(
        typ,
        types,
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        false,
    )
}

/// The bug this module exists to guard: a nested record whose every field is itself
/// default-constructible (literals only) must be emitted as `new {Type}()`, not `required`,
/// when the outer field's only default signal is a container-level `typed_default == Empty`.
#[test]
fn nested_record_fully_constructible_via_container_level_default_is_constructed_not_required() {
    let mut enabled = field("enabled", TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool));
    enabled.typed_default = Some(DefaultValue::BoolLiteral(true));
    let mut label = field("label", TypeRef::String);
    label.typed_default = Some(DefaultValue::StringLiteral("standard".to_string()));
    let mut nested = named_record_type("Settings", vec![enabled, label]);
    nested.has_default = true;

    let mut settings = field("settings", TypeRef::Named("Settings".to_string()));
    settings.typed_default = Some(DefaultValue::Empty);
    let mut typ = record_type(vec![settings]);
    typ.has_default = true;

    let code = render_with_types(&typ, std::slice::from_ref(&nested));

    assert!(
        code.contains("public Settings Settings { get; init; } = new Settings();"),
        "a fully default-constructible nested record must be emitted, not required:\n{code}"
    );
    assert!(
        !code.contains("required Settings"),
        "the nested record is constructible, so the outer field must not be `required`:\n{code}"
    );
}

/// Negative control: when the nested record itself has a field with no known default (so it
/// would render its own `required` member), the outer field must still fall back to `required`
/// rather than emitting a `new Settings()` that fails to compile with `CS9035`.
#[test]
fn nested_record_with_a_required_member_via_container_level_default_stays_required() {
    let nested = named_record_type("Settings", vec![field("label", TypeRef::String)]);

    let mut settings = field("settings", TypeRef::Named("Settings".to_string()));
    settings.typed_default = Some(DefaultValue::Empty);
    let mut typ = record_type(vec![settings]);
    typ.has_default = true;

    let nested_code = render_with_types(&nested, std::slice::from_ref(&nested));
    assert!(
        nested_code.contains("public required string Label { get; init; }"),
        "the fixture must actually declare a required member on the nested record:\n{nested_code}"
    );

    let code = render_with_types(&typ, std::slice::from_ref(&nested));

    assert!(
        code.contains("public required Settings Settings { get; init; }"),
        "a nested record that cannot itself be constructed must keep the outer field required, \
         never a `new Settings()` that fails to compile:\n{code}"
    );
}
