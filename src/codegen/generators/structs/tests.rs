use super::{
    gen_delegating_deserialize_impl, gen_struct, gen_struct_with_per_field_attrs, gen_struct_with_rename,
    struct_deserialize_delegation_field_sound, struct_wants_deserialize_delegation, type_needs_mutex,
    type_needs_tokio_mutex,
};
use crate::codegen::generators::{AsyncPattern, RustBindingConfig};
use crate::codegen::type_mapper::IdentityMapper;
use crate::core::ir::{
    CoreWrapper, FieldDef, MethodDef, PrimitiveType, ReceiverKind, SerdeContainerConversion, TypeDef, TypeRef,
};
use ahash::AHashSet;

fn method(name: &str, receiver: Option<ReceiverKind>, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.into(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver,
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn type_with_methods(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_path: format!("my_crate::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn tokio_mutex_when_all_refmut_methods_async() {
    let typ = type_with_methods(
        "WebSocketConnection",
        vec![
            method("send_text", Some(ReceiverKind::RefMut), true),
            method("receive_text", Some(ReceiverKind::RefMut), true),
            method("close", None, true),
        ],
    );
    assert!(type_needs_mutex(&typ));
    assert!(type_needs_tokio_mutex(&typ));
}

#[test]
fn no_tokio_mutex_when_any_refmut_is_sync() {
    let typ = type_with_methods(
        "Mixed",
        vec![
            method("async_op", Some(ReceiverKind::RefMut), true),
            method("sync_op", Some(ReceiverKind::RefMut), false),
        ],
    );
    assert!(type_needs_mutex(&typ));
    assert!(!type_needs_tokio_mutex(&typ));
}

#[test]
fn no_tokio_mutex_when_no_refmut() {
    let typ = type_with_methods("ReadOnly", vec![method("get", Some(ReceiverKind::Ref), true)]);
    assert!(!type_needs_mutex(&typ));
    assert!(!type_needs_tokio_mutex(&typ));
}

#[test]
fn no_tokio_mutex_when_empty_methods() {
    let typ = type_with_methods("Empty", vec![]);
    assert!(!type_needs_mutex(&typ));
    assert!(!type_needs_tokio_mutex(&typ));
}

// --- Deserialize-delegation eligibility and codegen -----------------------------------------

fn plain_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        optional: false,
        ..Default::default()
    }
}

fn container_conversion() -> SerdeContainerConversion {
    SerdeContainerConversion {
        from: Some("WireShape".to_string()),
        into: Some("WireShape".to_string()),
        try_from: None,
        transparent: false,
    }
}

fn type_with_fields(name: &str, fields: Vec<FieldDef>, conversion: SerdeContainerConversion) -> TypeDef {
    TypeDef {
        fields,
        is_opaque: false,
        serde_container_conversion: conversion,
        has_serde: true,
        ..type_with_methods(name, vec![])
    }
}

fn f64_field(name: &str) -> FieldDef {
    plain_field(name, TypeRef::Primitive(PrimitiveType::F64))
}

// Soundness matrix -----------------------------------------------------------------------------

#[test]
fn field_sound_true_for_two_field_primitive_pair() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], Default::default());
    assert!(struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_true_for_four_field_primitive_group() {
    let typ = type_with_fields(
        "Rect",
        vec![f64_field("a"), f64_field("b"), f64_field("c"), f64_field("d")],
        Default::default(),
    );
    assert!(struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_true_for_nested_struct_element() {
    let typ = type_with_fields(
        "Segment",
        vec![
            plain_field("start", TypeRef::Named("Point".to_string())),
            plain_field("end", TypeRef::Named("Point".to_string())),
        ],
        Default::default(),
    );
    assert!(struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_true_for_optional_element_mid_tuple() {
    let mut b = f64_field("b");
    b.optional = true;
    let typ = type_with_fields(
        "OptMid",
        vec![
            f64_field("a"),
            FieldDef {
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                ..b
            },
            f64_field("c"),
        ],
        Default::default(),
    );
    assert!(struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_false_for_unwrapped_opaque_field() {
    let typ = type_with_fields(
        "Holder",
        vec![plain_field("handle", TypeRef::Named("OpaqueHandle".to_string()))],
        Default::default(),
    );
    let opaque = vec!["OpaqueHandle".to_string()];
    assert!(!struct_deserialize_delegation_field_sound(&typ, &opaque, &[]));
}

#[test]
fn field_sound_true_for_opaque_field_marked_serializable() {
    let typ = type_with_fields(
        "Holder",
        vec![plain_field("handle", TypeRef::Named("OpaqueHandle".to_string()))],
        Default::default(),
    );
    let opaque = vec!["OpaqueHandle".to_string()];
    let serializable = vec!["OpaqueHandle".to_string()];
    assert!(struct_deserialize_delegation_field_sound(&typ, &opaque, &serializable));
}

#[test]
fn field_sound_false_for_sanitized_non_cow_field() {
    let mut field = f64_field("weird");
    field.sanitized = true;
    field.core_wrapper = CoreWrapper::None;
    let typ = type_with_fields("Sanitized", vec![field], Default::default());
    assert!(!struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_true_for_sanitized_cow_field() {
    let mut field = plain_field("label", TypeRef::String);
    field.sanitized = true;
    field.core_wrapper = CoreWrapper::Cow;
    let typ = type_with_fields("CowField", vec![field], Default::default());
    assert!(struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_false_for_cfg_gated_field() {
    let mut field = f64_field("feature_only");
    field.cfg = Some("feature = \"extra\"".to_string());
    let typ = type_with_fields("CfgGated", vec![field], Default::default());
    assert!(!struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn field_sound_false_when_type_has_stripped_cfg_fields() {
    let mut typ = type_with_fields("Stripped", vec![f64_field("x")], Default::default());
    typ.has_stripped_cfg_fields = true;
    assert!(!struct_deserialize_delegation_field_sound(&typ, &[], &[]));
}

#[test]
fn wants_delegation_false_when_mirror_reproduces_the_whole_serde_surface() {
    // No container conversion, no per-field default, no codec/flatten/skip -- the derived,
    // field-by-field `Deserialize` already agrees with the core type, so delegation is
    // unnecessary and must not fire.
    let typ = type_with_fields("Plain", vec![f64_field("x")], Default::default());
    assert!(!struct_wants_deserialize_delegation(&typ, &[], &[]));
}

#[test]
fn wants_delegation_true_with_from_into_and_sound_fields() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    assert!(struct_wants_deserialize_delegation(&typ, &[], &[]));
}

#[test]
fn wants_delegation_false_with_from_into_but_unsound_fields() {
    let typ = type_with_fields(
        "Holder",
        vec![plain_field("handle", TypeRef::Named("OpaqueHandle".to_string()))],
        container_conversion(),
    );
    let opaque = vec!["OpaqueHandle".to_string()];
    assert!(!struct_wants_deserialize_delegation(&typ, &opaque, &[]));
}

// Rendered-code assertions ----------------------------------------------------------------------

fn base_cfg<'a>() -> RustBindingConfig<'a> {
    RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &[],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "sample_core",
        async_pattern: AsyncPattern::None,
        has_serde: true,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
        source_crate_remaps: &[],
        emit_delegating_default_for_types: None,
        delegate_deserialize_to_core_for_types: None,
    }
}

/// Extracts the `#[derive(...)]` line so tests can assert on derive membership without being
/// fooled by "serde::Deserialize" also appearing inside the delegating impl's body text.
fn derive_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("rendered struct has a derive line")
}

#[test]
fn gen_struct_with_rename_delegates_for_sound_two_field_pair() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let mapper = IdentityMapper;
    let rendered = gen_struct_with_rename(&typ, &mapper, &cfg, |_| vec![], |_| None);

    assert!(
        !derive_line(&rendered).contains("serde::Deserialize"),
        "derive line must drop Deserialize when delegating: {rendered}"
    );
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Point {"),
        "expected a delegating Deserialize impl in: {rendered}"
    );
    assert!(
        rendered.contains("<my_crate::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegating impl must read the core type: {rendered}"
    );
    assert!(
        rendered.contains("#[derive(") && derive_line(&rendered).contains("serde::Serialize"),
        "Serialize stays derived (out of scope for this fix): {rendered}"
    );
}

#[test]
fn gen_struct_with_rename_keeps_derive_when_no_delegation_set_provided() {
    // Sound fields and a real container conversion, but the caller never proved a matching
    // `From<core::Type>` impl will exist (no delegation set) -- must NOT delegate.
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    let cfg = base_cfg();
    let mapper = IdentityMapper;
    let rendered = gen_struct_with_rename(&typ, &mapper, &cfg, |_| vec![], |_| None);

    assert!(derive_line(&rendered).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Point"));
}

#[test]
fn gen_struct_with_rename_keeps_derive_when_unsound_opaque_field() {
    let typ = type_with_fields(
        "Wrapper",
        vec![plain_field("handle", TypeRef::Named("OpaqueHandle".to_string()))],
        container_conversion(),
    );
    let delegatable: AHashSet<String> = ["Wrapper".to_string()].into_iter().collect();
    let opaque_names = ["OpaqueHandle".to_string()];
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        opaque_type_names: &opaque_names,
        ..base_cfg()
    };
    let mapper = IdentityMapper;
    let rendered = gen_struct_with_rename(&typ, &mapper, &cfg, |_| vec![], |_| None);

    // Falls back to the derived, field-by-field Deserialize -- the existing
    // SerdeContainerConversionUnsupported diagnostic keeps naming the real gap here.
    assert!(derive_line(&rendered).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Wrapper"));
}

#[test]
fn gen_struct_with_per_field_attrs_delegates_when_eligible() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let mapper = IdentityMapper;
    let rendered = gen_struct_with_per_field_attrs(&typ, &mapper, &cfg, |_| vec![]);

    assert!(!derive_line(&rendered).contains("serde::Deserialize"));
    assert!(rendered.contains("impl<'de> serde::Deserialize<'de> for Point {"));
}

#[test]
fn gen_struct_bare_delegates_when_eligible() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let mapper = IdentityMapper;
    let rendered = gen_struct(&typ, &mapper, &cfg);

    assert!(!derive_line(&rendered).contains("serde::Deserialize"));
    assert!(rendered.contains("impl<'de> serde::Deserialize<'de> for Point {"));
}

#[test]
fn gen_struct_bare_never_delegates_when_derive_already_agrees_with_core() {
    // Regression guard: a type whose serde surface the mirror reproduces in full must be
    // entirely unaffected, even if it happens to be named in the delegation set (e.g. by an
    // over-eager caller).
    let typ = type_with_fields("Plain", vec![f64_field("x")], Default::default());
    let delegatable: AHashSet<String> = ["Plain".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let mapper = IdentityMapper;
    let rendered = gen_struct(&typ, &mapper, &cfg);

    assert!(derive_line(&rendered).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Plain"));
}

#[test]
fn gen_delegating_deserialize_impl_applies_crate_remap() {
    let typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    let mut remapped = typ.clone();
    remapped.rust_path = "sample_core::geometry::Point".to_string();
    let rendered = gen_delegating_deserialize_impl(&remapped, "sample_core", "Js", &[("sample_core", "sample_wasm")]);

    assert!(rendered.contains("impl<'de> serde::Deserialize<'de> for JsPoint {"));
    assert!(rendered.contains("<sample_wasm::geometry::Point as serde::Deserialize>::deserialize"));
}

#[test]
fn gen_delegating_deserialize_impl_falls_back_to_core_import_for_bare_path() {
    let mut typ = type_with_fields("Point", vec![f64_field("x"), f64_field("y")], container_conversion());
    typ.rust_path = "Point".to_string();
    let rendered = gen_delegating_deserialize_impl(&typ, "sample_core", "", &[]);

    assert!(rendered.contains("<sample_core::Point as serde::Deserialize>::deserialize"));
}

// Per-field / container `#[serde(default)]` delegation -------------------------------------------
//
// These assert on the RENDERED struct, not on the predicate: a mirror that derives
// `Deserialize` field-by-field silently drops every `#[serde(default)]` the core type carries,
// so a partial JSON payload that the core type accepts is rejected by the binding. The only
// faithful answer is the delegating impl, which reads the core type's own `Deserialize`.

/// A field carrying a bare `#[serde(default)]` — `FieldDef::default == Some(...)`, which is
/// exactly what `extract_field` records for it.
fn field_with_serde_default(name: &str) -> FieldDef {
    FieldDef {
        default: Some("/* serde(default) */".to_string()),
        ..f64_field(name)
    }
}

#[test]
fn gen_struct_with_rename_delegates_for_field_with_bare_serde_default() {
    let typ = type_with_fields(
        "Config",
        vec![f64_field("threshold"), field_with_serde_default("timeout")],
        Default::default(),
    );
    let delegatable: AHashSet<String> = ["Config".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let rendered = gen_struct_with_rename(&typ, &IdentityMapper, &cfg, |_| vec![], |_| None);

    assert!(
        !derive_line(&rendered).contains("serde::Deserialize"),
        "a mirror that derives Deserialize drops the field's serde(default): {rendered}"
    );
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Config {"),
        "expected a delegating Deserialize impl in: {rendered}"
    );
    assert!(
        rendered.contains("<my_crate::Config as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegation must read the core type so its serde attrs are honoured: {rendered}"
    );
}

#[test]
fn gen_struct_with_per_field_attrs_delegates_for_field_with_serde_default_path() {
    let mut field = f64_field("retries");
    field.default = Some("serde(default = \"default_retries\")".to_string());
    let typ = type_with_fields("Retry", vec![field], Default::default());
    let delegatable: AHashSet<String> = ["Retry".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let rendered = gen_struct_with_per_field_attrs(&typ, &IdentityMapper, &cfg, |_| vec![]);

    assert!(!derive_line(&rendered).contains("serde::Deserialize"), "{rendered}");
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Retry {"),
        "{rendered}"
    );
}

#[test]
fn gen_struct_bare_delegates_for_container_level_serde_default() {
    let mut typ = type_with_fields("Settings", vec![f64_field("a"), f64_field("b")], Default::default());
    typ.serde_container_default = true;
    let delegatable: AHashSet<String> = ["Settings".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let rendered = gen_struct(&typ, &IdentityMapper, &cfg);

    assert!(!derive_line(&rendered).contains("serde::Deserialize"), "{rendered}");
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Settings {"),
        "{rendered}"
    );
}

// --- Per-field `#[serde(default)]` fallback when whole-type delegation cannot fire ------------
//
// Regression coverage for the `ExtractionConfig` bug: the core struct had one field
// (`mime_detection_policy`) carrying a bare `#[serde(default)]`, and a *different*, unrelated
// field referencing an opaque handle type (`ocr: Option<OcrConfig>`, mapped via `#[serde(skip)]`
// on the mirror). `struct_deserialize_delegation_field_sound` correctly refuses whole-type
// delegation because of the unrelated opaque field, which used to mean `mime_detection_policy`
// silently lost its `#[serde(default)]` too -- `ExtractionConfig.from_json("{}")` raised
// `ValueError: missing field 'mime_detection_policy'` even though the core type accepted the
// omission. These tests build the same shape (one opaque-blocked field, one sibling with a bare
// serde default) for each of the three struct generators and assert the sibling still gets
// `#[serde(default)]` on the derived, field-by-field `Deserialize` mirror.

fn opaque_field(name: &str) -> FieldDef {
    plain_field(name, TypeRef::Named("OpaqueHandle".to_string()))
}

#[test]
fn gen_struct_with_per_field_attrs_mirrors_serde_default_when_delegation_blocked_by_sibling_field() {
    let typ = type_with_fields(
        "ExtractionConfig",
        vec![opaque_field("ocr"), field_with_serde_default("mime_detection_policy")],
        Default::default(),
    );
    let delegatable: AHashSet<String> = ["ExtractionConfig".to_string()].into_iter().collect();
    let opaque = vec!["OpaqueHandle".to_string()];
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        opaque_type_names: &opaque,
        ..base_cfg()
    };
    let rendered = gen_struct_with_per_field_attrs(&typ, &IdentityMapper, &cfg, |_| vec![]);

    assert!(
        derive_line(&rendered).contains("serde::Deserialize"),
        "delegation must NOT fire while an unrelated field is opaque-blocked: {rendered}"
    );
    assert!(
        !rendered.contains("impl<'de> serde::Deserialize<'de> for ExtractionConfig {"),
        "no delegating impl expected: {rendered}"
    );
    assert!(
        rendered.contains("#[serde(default)]") && rendered.contains("pub mime_detection_policy"),
        "the sibling field must keep its own serde(default) on the derived mirror: {rendered}"
    );
}

#[test]
fn gen_struct_with_rename_mirrors_serde_default_when_delegation_blocked_by_sibling_field() {
    let typ = type_with_fields(
        "ExtractionConfig",
        vec![opaque_field("ocr"), field_with_serde_default("mime_detection_policy")],
        Default::default(),
    );
    let delegatable: AHashSet<String> = ["ExtractionConfig".to_string()].into_iter().collect();
    let opaque = vec!["OpaqueHandle".to_string()];
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        opaque_type_names: &opaque,
        ..base_cfg()
    };
    let rendered = gen_struct_with_rename(&typ, &IdentityMapper, &cfg, |_| vec![], |_| None);

    assert!(
        derive_line(&rendered).contains("serde::Deserialize"),
        "delegation must NOT fire while an unrelated field is opaque-blocked: {rendered}"
    );
    assert!(
        rendered.contains("#[serde(default)]") && rendered.contains("pub mime_detection_policy"),
        "the sibling field must keep its own serde(default) on the derived mirror: {rendered}"
    );
}

#[test]
fn gen_struct_bare_mirrors_serde_default_path_when_delegation_not_requested() {
    let mut field = f64_field("retries");
    field.default = Some("serde(default = \"default_retries\")".to_string());
    let typ = type_with_fields("Retry", vec![field], Default::default());
    // No `delegate_deserialize_to_core_for_types` entry for "Retry" -- delegation is never even
    // attempted for this type in this run.
    let cfg = base_cfg();
    let rendered = gen_struct(&typ, &IdentityMapper, &cfg);

    assert!(derive_line(&rendered).contains("serde::Deserialize"), "{rendered}");
    assert!(
        rendered.contains("serde(default = \"default_retries\")"),
        "a `default = \"path\"` field must also be mirrored verbatim: {rendered}"
    );
}

// --- Regression: a field-level valued `#[serde(default = "path")]` must not be duplicated ----
//
// 0.82.1 (#305, "keep serde defaults on mirror fields") added the per-field mirroring above
// unconditionally: it pushed the core field's own `#[serde(default...)]` even when the caller's
// `extra_field_attrs` (php's `field_attrs_fn`, which resolves a per-field default function via
// `serde_defaults::serde_default_fn_name` and pushes a valued `serde(default = "path")` itself)
// had already written one. serde rejects two `#[serde(default...)]` attributes on one field
// outright ("duplicate serde attribute `default`"), which is exactly what shipped as the
// crawlberg-php / xberg-php regression: `capture_network_events` carried both
// `serde(default = "crate::serde_defaults::browser_config_capture_network_events")` from the
// backend's own attribute and a bare `serde(default)` from this fallback. These tests build the
// same shape -- a bare core default plus a caller-supplied valued default on the same field --
// for each of the three struct generators and assert exactly one `serde(default...)` survives.

#[test]
fn gen_struct_with_per_field_attrs_keeps_only_the_valued_default_already_emitted_by_the_caller() {
    let typ = type_with_fields(
        "BrowserConfig",
        vec![field_with_serde_default("capture_network_events")],
        Default::default(),
    );
    let cfg = base_cfg();
    let rendered = gen_struct_with_per_field_attrs(&typ, &IdentityMapper, &cfg, |_| {
        vec!["serde(default = \"crate::serde_defaults::browser_config_capture_network_events\")".to_string()]
    });

    let default_attr_count = rendered.matches("serde(default").count();
    assert_eq!(
        default_attr_count, 1,
        "exactly one serde(default...) attribute must survive on the field, got {default_attr_count}: {rendered}"
    );
    assert!(
        rendered.contains("serde(default = \"crate::serde_defaults::browser_config_capture_network_events\")"),
        "the valued default from extra_field_attrs must win over the bare fallback: {rendered}"
    );
    assert!(
        !rendered.contains("#[serde(default)]"),
        "the bare fallback default must not also be emitted: {rendered}"
    );
}

#[test]
fn gen_struct_with_rename_keeps_only_the_valued_default_already_emitted_by_the_caller() {
    let typ = type_with_fields(
        "BrowserConfig",
        vec![field_with_serde_default("capture_network_events")],
        Default::default(),
    );
    let cfg = base_cfg();
    let rendered = gen_struct_with_rename(
        &typ,
        &IdentityMapper,
        &cfg,
        |_| vec!["serde(default = \"crate::serde_defaults::browser_config_capture_network_events\")".to_string()],
        |_| None,
    );

    let default_attr_count = rendered.matches("serde(default").count();
    assert_eq!(
        default_attr_count, 1,
        "exactly one serde(default...) attribute must survive on the field, got {default_attr_count}: {rendered}"
    );
    assert!(!rendered.contains("#[serde(default)]"), "{rendered}");
}

#[test]
fn gen_struct_bare_keeps_only_the_valued_default_already_present_in_field_attrs() {
    let typ = type_with_fields(
        "BrowserConfig",
        vec![field_with_serde_default("capture_network_events")],
        Default::default(),
    );
    let field_attrs = ["serde(default = \"crate::serde_defaults::browser_config_capture_network_events\")"];
    let cfg = RustBindingConfig {
        field_attrs: &field_attrs,
        ..base_cfg()
    };
    let rendered = gen_struct(&typ, &IdentityMapper, &cfg);

    let default_attr_count = rendered.matches("serde(default").count();
    assert_eq!(
        default_attr_count, 1,
        "exactly one serde(default...) attribute must survive on the field, got {default_attr_count}: {rendered}"
    );
    assert!(!rendered.contains("#[serde(default)]"), "{rendered}");
}

#[test]
fn gen_struct_with_rename_delegates_for_field_with_serde_with_codec() {
    let mut field = f64_field("elapsed");
    field.serde_with = Some("humantime_serde".to_string());
    let typ = type_with_fields("Timing", vec![field], Default::default());
    let delegatable: AHashSet<String> = ["Timing".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let rendered = gen_struct_with_rename(&typ, &IdentityMapper, &cfg, |_| vec![], |_| None);

    assert!(
        !derive_line(&rendered).contains("serde::Deserialize"),
        "the mirror never re-emits `#[serde(with = ...)]`, so its derive reads the wrong wire shape: {rendered}"
    );
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Timing {"),
        "{rendered}"
    );
}

/// Positive control: a struct whose every serde-relevant attribute the mirror DOES reproduce
/// (here `#[serde(rename = ...)]`, which `gen_struct_with_rename` re-emits verbatim) must keep
/// deriving `Deserialize` directly and must NOT pick up a delegating impl. Widening the
/// delegation trigger must not sweep in types the derived impl already gets right.
#[test]
fn gen_struct_with_rename_keeps_derive_when_no_unreproducible_serde_attrs() {
    let mut renamed = f64_field("tool_type");
    renamed.serde_rename = Some("type".to_string());
    let typ = type_with_fields("Reproducible", vec![f64_field("x"), renamed], Default::default());
    let delegatable: AHashSet<String> = ["Reproducible".to_string()].into_iter().collect();
    let cfg = RustBindingConfig {
        delegate_deserialize_to_core_for_types: Some(&delegatable),
        ..base_cfg()
    };
    let rendered = gen_struct_with_rename(&typ, &IdentityMapper, &cfg, |_| vec![], |_| None);

    assert!(
        derive_line(&rendered).contains("serde::Deserialize"),
        "nothing here disagrees with the derive, so it must stay derived: {rendered}"
    );
    assert!(
        !rendered.contains("impl<'de> serde::Deserialize<'de> for Reproducible"),
        "must not emit a delegating impl for a faithfully-mirrored struct: {rendered}"
    );
    assert!(
        rendered.contains("serde(rename = \"type\")"),
        "the reproduced attribute must still be emitted: {rendered}"
    );
}
