use super::gen_from_binding_to_core;
use super::gen_from_binding_to_core_cfg;
use super::gen_from_lifetime_type_constructor;
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, MethodDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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

fn boxed_named_field(name: &str, optional: bool, core_wrapper: CoreWrapper) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty: TypeRef::Named("Inner".to_string()),
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: true,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// Regression guard: a plain (non-opaque) `Option<Box<Inner>>` struct field must keep
/// generating `.map(Into::into).map(Box::new)` — this is the path every generated binding
/// relies on today (e.g. `Option<Box<ExtractedDocument>>`), so it must not change shape
/// while fixing the opaque+boxed combination.
#[test]
fn boxed_named_field_optional_transparent_path_unchanged() {
    let typ = type_with_field(boxed_named_field("child", true, CoreWrapper::None));

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("child: val.child.map(Into::into).map(Box::new)"),
        "expected the existing transparent Option<Box<T>> shape to be preserved; got:\n{out}"
    );
}

/// Companion to the above for the non-optional `Box<Inner>` shape.
#[test]
fn boxed_named_field_non_optional_transparent_path_unchanged() {
    let typ = type_with_field(boxed_named_field("child", false, CoreWrapper::None));

    let out = gen_from_binding_to_core(&typ, "crate");

    assert!(
        out.contains("child: Box::new(val.child.into())"),
        "expected the existing transparent Box<T> shape to be preserved; got:\n{out}"
    );
}

/// Regression: an opaque `Arc`-wrapper field (`OpaqueHandle { inner: Arc<T> }`) that is also
/// `Box<T>` on the core struct must deref-clone the shared value and rebox it, not move the
/// `Arc<T>` out directly — moving it would produce `Option<Arc<T>>` where `Option<Box<T>>`
/// is required.
#[test]
fn boxed_opaque_arc_field_optional_reboxes_instead_of_moving_arc() {
    let opaque_type_name = "Inner".to_string();
    let mut opaque_set = AHashSet::new();
    opaque_set.insert(opaque_type_name);

    let field = boxed_named_field("child", true, CoreWrapper::Arc);
    let config = ConversionConfig {
        opaque_types: Some(&opaque_set),
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

    assert!(
        out.contains("child: val.child.map(|v| Box::new((*v.inner).clone()))"),
        "expected the opaque Arc handle to be deref-cloned and reboxed, got:\n{out}"
    );
    assert!(
        !out.contains("val.child.map(|v| v.inner)"),
        "must not move the bare Arc<T> handle into a Box<T> field, got:\n{out}"
    );
}

/// Non-optional companion: `Box<Inner>` sourced from an opaque `Arc`-wrapper field.
#[test]
fn boxed_opaque_arc_field_non_optional_reboxes_instead_of_moving_arc() {
    let opaque_type_name = "Inner".to_string();
    let mut opaque_set = AHashSet::new();
    opaque_set.insert(opaque_type_name);

    let field = boxed_named_field("child", false, CoreWrapper::Arc);
    let config = ConversionConfig {
        opaque_types: Some(&opaque_set),
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

    assert!(
        out.contains("child: Box::new((*val.child.inner).clone())"),
        "expected the opaque Arc handle to be deref-cloned and reboxed, got:\n{out}"
    );
    assert!(
        !out.contains("child: val.child.inner"),
        "must not move the bare Arc<T> handle into a Box<T> field, got:\n{out}"
    );
}

/// Regression: a trait-bridge, no-wrapper opaque field (`core_wrapper == None`, resolved via
/// `trait_bridge_arc_wrapper_field_names`) that is also `Box<T>` on the core struct must
/// rebox the cloned value rather than emitting a bare (un-boxed) clone.
#[test]
fn boxed_opaque_no_wrapper_trait_bridge_field_reboxes_clone() {
    let opaque_type_name = "Inner".to_string();
    let mut opaque_set = AHashSet::new();
    opaque_set.insert(opaque_type_name);
    let arc_wrapper = vec!["child".to_string()];

    let field = boxed_named_field("child", true, CoreWrapper::None);
    let config = ConversionConfig {
        opaque_types: Some(&opaque_set),
        trait_bridge_arc_wrapper_field_names: &arc_wrapper,
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

    assert!(
        out.contains("child: val.child.map(|v| Box::new((*v.inner).clone()))"),
        "expected the trait-bridge clone to be reboxed for a Box<T> field, got:\n{out}"
    );
}

fn enum_named_field(name: &str, type_name: &str, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        optional,
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// A `has_lifetime_params` type (private fields, constructed via a static method) with a
/// single field, matching the shape `gen_from_lifetime_type_constructor` targets.
fn lifetime_type_with_enum_field(field: FieldDef) -> TypeDef {
    let mut typ = type_with_field(field.clone());
    typ.has_lifetime_params = true;
    typ.methods.push(MethodDef {
        name: "with_owned".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: field.name.clone(),
            ty: field.ty.clone(),
            optional: field.optional,
            ..crate::core::ir::ParamDef::default()
        }],
        return_type: TypeRef::Named(typ.name.clone()),
        is_static: true,
        receiver: None,
        cfg: None,
        ..MethodDef::default()
    });
    typ
}

/// Regression for a PHP process crash: a PHP-facing enum-as-`String` field can hold any value
/// a script assigns before the binding->core conversion re-parses it, and `.expect()` on a
/// bad value used to panic *inside* a `From` impl called from generated FFI code -- unwinding
/// across the boundary into Zend's C frames (undefined behaviour, not a catchable PHP
/// exception). The conversion must now report the failure via `PhpException::throw()` and
/// fall back to a known-safe variant instead of panicking. ~keep
#[test]
fn lifetime_type_enum_string_field_reports_php_exception_instead_of_panicking() {
    let field = enum_named_field("node_type", "NodeType", false);
    let typ = lifetime_type_with_enum_field(field);

    let mut enum_names = AHashSet::new();
    enum_names.insert("NodeType".to_string());
    let mut fallback = AHashMap::new();
    fallback.insert("NodeType".to_string(), "Text".to_string());
    let config = ConversionConfig {
        enum_string_names: Some(&enum_names),
        enum_string_fallback_variant: Some(&fallback),
        ..ConversionConfig::default()
    };

    let out = gen_from_lifetime_type_constructor(
        &typ,
        "html_to_markdown_rs::NodeContext",
        "NodeContext",
        "html_to_markdown_rs",
        &config,
    )
    .expect("constructor call must be generated");

    assert!(
        !out.contains(".expect(\"valid NodeType\")"),
        "a bad PHP-assigned enum string must no longer panic across the FFI boundary, got:\n{out}"
    );
    assert!(
        out.contains("ext_php_rs::exception::PhpException::default") && out.contains(".throw()"),
        "the parse failure must be reported to PHP as a catchable exception, got:\n{out}"
    );
    assert!(
        out.contains("html_to_markdown_rs::NodeType::Text"),
        "the conversion must still return a valid Self via the known fallback variant, got:\n{out}"
    );
}

/// Companion regression, optional-field shape: `None` is always a valid fallback for an
/// `Option<Enum>` field, so this path never needs a known fallback variant at all -- but a
/// parse failure must still be surfaced to PHP rather than silently discarded.
#[test]
fn lifetime_type_optional_enum_string_field_falls_back_to_none_and_reports_exception() {
    let field = enum_named_field("node_type", "NodeType", true);
    let typ = lifetime_type_with_enum_field(field);

    let mut enum_names = AHashSet::new();
    enum_names.insert("NodeType".to_string());
    let config = ConversionConfig {
        enum_string_names: Some(&enum_names),
        ..ConversionConfig::default()
    };

    let out = gen_from_lifetime_type_constructor(
        &typ,
        "html_to_markdown_rs::NodeContext",
        "NodeContext",
        "html_to_markdown_rs",
        &config,
    )
    .expect("constructor call must be generated");

    assert!(
        !out.contains(".expect(\"valid NodeType\")"),
        "an optional field never needs the panicking path -- None is always a valid fallback, got:\n{out}"
    );
    assert!(
        out.contains("ext_php_rs::exception::PhpException::default") && out.contains(".throw()"),
        "a parse failure on an optional field must still be reported to PHP, not silently dropped, got:\n{out}"
    );
    assert!(
        out.contains("None"),
        "the optional field must fall back to None on a parse failure, got:\n{out}"
    );
}

/// Positive control: when no fallback variant is known for the colliding enum (absent from
/// `enum_string_fallback_variant`), the generator must not fabricate an unsound placeholder --
/// it keeps the original panicking expression rather than reference a variant name it cannot
/// verify exists.
#[test]
fn lifetime_type_enum_string_field_without_known_fallback_keeps_original_panic() {
    let field = enum_named_field("node_type", "NodeType", false);
    let typ = lifetime_type_with_enum_field(field);

    let mut enum_names = AHashSet::new();
    enum_names.insert("NodeType".to_string());
    let config = ConversionConfig {
        enum_string_names: Some(&enum_names),
        enum_string_fallback_variant: None,
        ..ConversionConfig::default()
    };

    let out = gen_from_lifetime_type_constructor(
        &typ,
        "html_to_markdown_rs::NodeContext",
        "NodeContext",
        "html_to_markdown_rs",
        &config,
    )
    .expect("constructor call must be generated");

    assert!(
        out.contains(".expect(\"valid NodeType\")"),
        "without a known fallback variant, the original (panicking) expression must be kept \
         rather than fabricate an unsound placeholder, got:\n{out}"
    );
}

/// A `#[cfg(...)]` on a bare assignment statement is `E0658` -- attributes on expressions are
/// unstable, and `__result.field = value;` is an expression statement. The gate has to sit on a
/// block, which is a stable place for one. Emitting it bare produced 19 compile errors in one
/// consumer's NAPI crate, in generated code that no unit test covered because every other backend
/// takes the struct-literal branch instead.
#[test]
fn a_gated_optionalized_field_wraps_its_assignment_in_a_block() {
    let mut field = FieldDef {
        name: "extracted_keywords".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        ..FieldDef::default()
    };
    field.cfg = Some(r#"feature = "keywords""#.to_string());
    let typ = TypeDef {
        name: "Document".to_string(),
        rust_path: "test_lib::Document".to_string(),
        fields: vec![field],
        has_default: true,
        ..TypeDef::default()
    };
    let config = ConversionConfig {
        optionalize_defaults: true,
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&typ, "test_lib", &config);

    assert!(
        out.contains(r#"#[cfg(feature = "keywords")]"#),
        "the field's gate must still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("#[cfg(feature = \"keywords\")]\n        __result."),
        "the gate must not sit directly on a bare assignment statement (E0658), got:\n{out}"
    );
    assert!(
        out.contains("#[cfg(feature = \"keywords\")]\n        {"),
        "the gated assignment must be wrapped in a block, got:\n{out}"
    );
}

/// CONTROL: an ungated field's assignment must stay a bare statement -- a fix that wrapped every
/// assignment in a block would pass the test above while needlessly changing every other line.
#[test]
fn an_ungated_optionalized_field_keeps_a_bare_assignment() {
    let typ = TypeDef {
        name: "Document".to_string(),
        rust_path: "test_lib::Document".to_string(),
        fields: vec![FieldDef {
            name: "title".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            ..FieldDef::default()
        }],
        has_default: true,
        ..TypeDef::default()
    };
    let config = ConversionConfig {
        optionalize_defaults: true,
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&typ, "test_lib", &config);

    assert!(!out.contains("#[cfg("), "an ungated field must emit no gate, got:\n{out}");
    assert!(
        !out.contains("        {\n"),
        "an ungated assignment must not be wrapped in a block, got:\n{out}"
    );
}

/// Regression: `declared_features` must narrow a struct-literal field's copied `#[cfg(...)]`
/// gate to only the feature names this binding crate declares -- a real PHP consumer's crate
/// declared "url-config-types" but not "url-ingestion", and the verbatim `any(...)` gate
/// this direction copies onto `val.crawl.into()` triggered `unexpected_cfg_condition_value`
/// under `-D warnings`.
#[test]
fn gate_with_one_undeclared_feature_narrows_to_the_declared_term_alone_binding_to_core() {
    let field = FieldDef {
        name: "crawl".to_string(),
        ty: TypeRef::String,
        cfg: Some(r#"any(feature = "url-ingestion", feature = "url-config-types")"#.to_string()),
        ..FieldDef::default()
    };
    let never_skip = vec!["crawl".to_string()];
    let declared: std::collections::HashSet<&str> = ["url-config-types"].into_iter().collect();
    let config = ConversionConfig {
        never_skip_cfg_field_names: &never_skip,
        strip_cfg_fields_from_binding_struct: true,
        declared_features: Some(&declared),
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "test_lib", &config);

    let crawl_line = out
        .lines()
        .find(|line| line.contains("crawl:"))
        .expect("crawl field initialiser present");
    let cfg_line = out
        .lines()
        .take_while(|line| !line.contains("crawl:"))
        .last()
        .expect("a preceding cfg attribute line exists");

    assert!(
        cfg_line.contains(r#"#[cfg(feature = "url-config-types")]"#),
        "gate must narrow to the single declared term, got cfg line:\n{cfg_line}\nfull output:\n{out}"
    );
    assert!(
        !cfg_line.contains("url-ingestion"),
        "the undeclared feature must not appear in the emitted cfg attribute, got:\n{cfg_line}"
    );
    assert!(crawl_line.contains("val.crawl"), "field initialiser missing, got:\n{out}");
}

/// CONTROL for the regression above: when every named feature is declared, the gate must be
/// copied verbatim, byte for byte.
#[test]
fn gate_with_every_feature_declared_is_emitted_unchanged_binding_to_core() {
    let field = FieldDef {
        name: "crawl".to_string(),
        ty: TypeRef::String,
        cfg: Some(r#"any(feature = "url-ingestion", feature = "url-config-types")"#.to_string()),
        ..FieldDef::default()
    };
    let never_skip = vec!["crawl".to_string()];
    let declared: std::collections::HashSet<&str> = ["url-ingestion", "url-config-types"].into_iter().collect();
    let config = ConversionConfig {
        never_skip_cfg_field_names: &never_skip,
        strip_cfg_fields_from_binding_struct: true,
        declared_features: Some(&declared),
        ..ConversionConfig::default()
    };

    let out = gen_from_binding_to_core_cfg(&type_with_field(field), "test_lib", &config);

    assert!(
        out.contains(r#"#[cfg(any(feature = "url-ingestion", feature = "url-config-types"))]"#),
        "a gate whose every feature is declared must be copied verbatim, got:\n{out}"
    );
}
