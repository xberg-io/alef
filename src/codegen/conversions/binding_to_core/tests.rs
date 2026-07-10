use super::gen_from_binding_to_core;
use super::gen_from_binding_to_core_cfg;
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;

fn type_with_field(field: FieldDef) -> TypeDef {
    TypeDef {
        name: "ProcessConfig".to_string(),
        rust_path: "crate::ProcessConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![field],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn sanitized_cow_string_field_converts_to_core() {
    let field = FieldDef {
        name: "language".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: true,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(DefaultValue::Empty),
        core_wrapper: CoreWrapper::Cow,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let out = gen_from_binding_to_core(&type_with_field(field), "crate");

    assert!(out.contains("language: val.language.into()"));
    assert!(!out.contains("language: Default::default()"));
}

#[test]
fn binding_excluded_cfg_field_is_not_emitted_into_core_literal() {
    let field = FieldDef {
        name: "di_container".to_string(),
        ty: TypeRef::String,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: Some("feature = \"di\"".to_string()),
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: true,
        binding_exclusion_reason: Some("internal implementation detail".to_string()),
        original_type: None,
    };
    let mut typ = type_with_field(field);
    typ.has_stripped_cfg_fields = true;

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        !out.contains("di_container:"),
        "cfg-gated binding-excluded fields may not exist in the core struct; got:\n{out}"
    );
    assert!(
        out.contains("..Default::default()"),
        "stripped cfg fields should be filled by the default update; got:\n{out}"
    );
}

/// Trait-bridge OptionsField field with Arc wrapper: the binding→core From impl must
/// emit `val.visitor.map(|v| (*v.inner).clone())` and must NOT fall back to
/// `visitor: Default::default()`, which would silently drop the visitor handle.
#[test]
fn trait_bridge_arc_wrapper_field_forwards_value_not_default() {
    let opaque_type_name = "VisitorHandle".to_string();
    let mut opaque_set = AHashSet::new();
    opaque_set.insert(opaque_type_name.clone());

    let field = FieldDef {
        name: "visitor".to_string(),
        ty: TypeRef::Named(opaque_type_name.clone()),
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: Some("feature = \"visitor\"".to_string()),
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let never_skip = vec!["visitor".to_string()];
    let arc_wrapper = vec!["visitor".to_string()];

    let config = ConversionConfig {
        opaque_types: Some(&opaque_set),
        never_skip_cfg_field_names: &never_skip,
        trait_bridge_arc_wrapper_field_names: &arc_wrapper,
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

    assert!(
        out.contains("val.visitor.map(|v| (*v.inner).clone())"),
        "expected arc-wrapper clone forwarding, got:\n{out}"
    );
    assert!(
        !out.contains("visitor: Default::default()"),
        "must not emit Default::default() for arc-wrapper trait-bridge field, got:\n{out}"
    );
}

/// When `trait_bridge_arc_wrapper_field_names` is empty (default), the old
/// `Default::default()` fallback is preserved for opaque-no-wrapper fields.
#[test]
fn opaque_no_wrapper_field_without_arc_flag_emits_default() {
    let opaque_type_name = "OpaqueHandle".to_string();
    let mut opaque_set = AHashSet::new();
    opaque_set.insert(opaque_type_name.clone());

    let field = FieldDef {
        name: "handle".to_string(),
        ty: TypeRef::Named(opaque_type_name.clone()),
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
    };

    let config = ConversionConfig {
        opaque_types: Some(&opaque_set),
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

    assert!(
        out.contains("handle: Default::default()"),
        "expected Default::default() for non-arc-wrapper opaque field, got:\n{out}"
    );
    assert!(
        !out.contains("(*val.handle.inner).clone()"),
        "must not emit arc-clone for non-arc-wrapper opaque field, got:\n{out}"
    );
}

/// Regression: a binding-excluded field (with no cfg gate) must not be emitted as
/// `field: Default::default()` because that calls the SUB-type's Default and
/// bypasses any core-type Default override. The output must skip the field and
/// emit `..Default::default()` so the field is filled from the core type's
/// `Default` impl instead.
///
/// Pattern that motivates this: a top-level config field of type `SubPolicy` is
/// `binding_excluded` because `SubPolicy` carries a `#[serde(skip)]
/// HashSet<&'static str>` that cannot cross a JSON boundary. Emitting
/// `field: Default::default()` calls `SubPolicy::default()` directly, bypassing
/// the parent `Config::default()` which might read an environment variable to
/// pick a non-stricter setting. `..Default::default()` delegates to the parent
/// `Config::default()` so its bespoke initialization runs.
#[test]
fn binding_excluded_non_cfg_field_falls_through_to_core_default_trailer() {
    let field = FieldDef {
        name: "ssrf".to_string(),
        ty: TypeRef::Named("SsrfPolicy".to_string()),
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
        binding_excluded: true,
        binding_exclusion_reason: Some("contains non-serializable scheme_allowlist".to_string()),
        original_type: None,
    };
    let typ = type_with_field(field);

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        !out.contains("ssrf: Default::default()"),
        "binding-excluded field must not be emitted with field-level Default::default(); got:\n{out}"
    );
    assert!(
        out.contains("..Default::default()"),
        "binding-excluded fields require the core-type Default trailer; got:\n{out}"
    );
}

/// Regression: when a core type has `binding_excluded` fields but does NOT
/// implement `Default`, the spread trailer `..Default::default()` will not
/// compile. In that case the From impl must fall back to per-field
/// `Default::default()` for each excluded field — there is no bespoke core
/// `Default` whose semantics could be bypassed (and the alternative is a
/// generated impl that does not compile).
///
/// Pattern that motivates this: a core type whose internal field is annotated
/// `#[cfg_attr(alef, alef(skip))]` to keep it off the binding wire, but the
/// struct itself has no `Default` impl. Previously the From impl emitted
/// `..Default::default()` and failed with `E0277: the trait bound 'T: Default'
/// is not satisfied`.
#[test]
fn binding_excluded_field_on_type_without_default_uses_per_field_fallback() {
    let field = FieldDef {
        name: "cursor".to_string(),
        ty: TypeRef::Named("Cursor".to_string()),
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
        binding_excluded: true,
        binding_exclusion_reason: Some("internal read cursor".to_string()),
        original_type: None,
    };
    let mut typ = type_with_field(field);
    typ.has_default = false;
    typ.has_stripped_cfg_fields = false;

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("cursor: Default::default()"),
        "binding-excluded field on a type without `Default` must fall back to \
         per-field `Default::default()`; got:\n{out}"
    );
    assert!(
        !out.contains("..Default::default()"),
        "the spread trailer must not be emitted when the core type does not \
         derive/impl Default — it would fail to compile (E0277); got:\n{out}"
    );
}

fn string_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
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

/// A core type with private (`pub(crate)`) fields cannot be built with struct-literal
/// syntax from a foreign crate. When it derives `Default`, the conversion must seed a
/// `T::default()` base (which fills the private fields) and assign only the public fields.
#[test]
fn private_fields_type_with_default_uses_builder() {
    let mut typ = type_with_field(string_field("content"));
    typ.has_private_fields = true;
    typ.has_default = true;
    typ.has_serde = true;

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("crate::ProcessConfig::default()"),
        "builder must seed the core Default to fill private fields; got:\n{out}"
    );
    assert!(
        out.contains("__result.content = "),
        "builder must assign public fields onto the default base; got:\n{out}"
    );
    assert!(
        !out.contains("content: val.content"),
        "must not emit a struct-literal field for a type with private fields; got:\n{out}"
    );
}

/// A core type with private fields and NO `Default` impl cannot be constructed by the
/// builder strategy (no base to seed) and a struct literal is impossible (private fields).
/// The generator must emit a guiding `compile_error!` rather than broken code — even when
/// the type derives serde, because per-field serde construction is fragile (`into()` target
/// ambiguity). The contract is: derive `Default` (or expose a constructor) on such a type.
#[test]
fn private_fields_type_without_default_emits_compile_error() {
    let mut typ = type_with_field(string_field("content"));
    typ.has_private_fields = true;
    typ.has_default = false;
    typ.has_serde = true;

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("compile_error!"),
        "a private-field type without Default must emit a guiding compile_error!; got:\n{out}"
    );
    assert!(
        out.contains("Default"),
        "the compile_error must guide the author to derive Default; got:\n{out}"
    );
    assert!(
        !out.contains("content: val.content"),
        "must not emit a struct-literal field for a type with private fields; got:\n{out}"
    );
}

/// Forward-compatibility: a fully-mirrored core type that implements `Default`
/// (every field present in the binding, none binding-excluded, no cfg-stripping)
/// must still get the `..Default::default()` trailer. Without it the exhaustive
/// literal stops compiling with E0063 the moment an additive field lands on the
/// core struct, until the bindings are regenerated.
#[test]
fn fully_mirrored_type_with_default_emits_spread_trailer() {
    let typ = type_with_field(string_field("content"));

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("content: val.content"),
        "mirrored fields must still be assigned explicitly; got:\n{out}"
    );
    assert!(
        out.contains("..Default::default()"),
        "a has_default core type must get the spread trailer so an additive core \
         field falls back to its default instead of breaking the impl; got:\n{out}"
    );
    assert!(
        out.contains("#[allow(clippy::needless_update)]"),
        "the spread over a fully-mirrored literal needs the needless_update allow; got:\n{out}"
    );
}

/// Companion: a fully-mirrored core type WITHOUT `Default` cannot take the spread
/// trailer (E0277) — the exhaustive literal must stay as-is.
#[test]
fn fully_mirrored_type_without_default_keeps_exhaustive_literal() {
    let mut typ = type_with_field(string_field("content"));
    typ.has_default = false;

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("content: val.content"),
        "mirrored fields must be assigned explicitly; got:\n{out}"
    );
    assert!(
        !out.contains("..Default::default()"),
        "the spread trailer must not be emitted when the core type has no Default \
         impl — it would fail to compile (E0277); got:\n{out}"
    );
}
