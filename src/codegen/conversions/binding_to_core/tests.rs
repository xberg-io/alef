use super::gen_from_binding_to_core;
use super::gen_from_binding_to_core_cfg;
use super::gen_from_lifetime_type_constructor;
use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::config::WasmCamelRecasedEnum;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

pub(super) fn type_with_field(field: FieldDef) -> TypeDef {
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

pub(super) fn wasm_required_tagged_config(tagged_names: &AHashSet<String>) -> ConversionConfig<'_> {
    ConversionConfig {
        type_name_prefix: "Wasm",
        map_uses_jsvalue: true,
        tagged_data_enum_names: Some(tagged_names),
        wasm_required_tagged_enum_decode_throws: true,
        option_duration_on_defaults: true,
        ..Default::default()
    }
}

pub(super) fn required_authentication_field(boxed: bool) -> FieldDef {
    FieldDef {
        name: "authentication".to_string(),
        ty: TypeRef::Named("Authentication".to_string()),
        is_boxed: boxed,
        ..Default::default()
    }
}

#[test]
fn wasm_required_tagged_decode_is_structural_across_owner_construction_paths() {
    let tagged_names: AHashSet<String> = ["Authentication".to_string()].into_iter().collect();
    let config = wasm_required_tagged_config(&tagged_names);

    let mut ordinary = type_with_field(required_authentication_field(false));
    ordinary.has_default = false;
    let ordinary_out = gen_from_binding_to_core_cfg(&ordinary, "sample_core", &config);
    assert!(
        ordinary_out.contains("authentication: serde_wasm_bindgen::from_value"),
        "{ordinary_out}"
    );

    let mut boxed = ordinary.clone();
    boxed.fields[0].is_boxed = true;
    let boxed_out = gen_from_binding_to_core_cfg(&boxed, "sample_core", &config);
    assert!(
        boxed_out.contains("authentication: Box::new(serde_wasm_bindgen::from_value"),
        "{boxed_out}"
    );

    let mut private_default = ordinary.clone();
    private_default.has_default = true;
    private_default.has_private_fields = true;
    let private_out = gen_from_binding_to_core_cfg(&private_default, "sample_core", &config);
    assert!(
        private_out.contains("let mut __result = crate::ProcessConfig::default()"),
        "{private_out}"
    );
    assert!(
        private_out.contains("__result.authentication = serde_wasm_bindgen::from_value"),
        "{private_out}"
    );

    let mut default_seeded = ordinary.clone();
    default_seeded.has_default = true;
    default_seeded.fields.push(FieldDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
        ..Default::default()
    });
    let default_seeded_out = gen_from_binding_to_core_cfg(&default_seeded, "sample_core", &config);
    assert!(
        default_seeded_out.contains("let mut __result = crate::ProcessConfig::default()"),
        "{default_seeded_out}"
    );
    assert!(
        default_seeded_out.contains("__result.authentication = serde_wasm_bindgen::from_value"),
        "{default_seeded_out}"
    );

    let mut explicit_constructor = ordinary;
    explicit_constructor.methods.push(MethodDef {
        name: "new".to_string(),
        params: vec![ParamDef {
            name: "authentication".to_string(),
            ty: TypeRef::Named("Authentication".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Named("ProcessConfig".to_string()),
        is_static: true,
        ..Default::default()
    });
    let constructor_out = gen_from_binding_to_core_cfg(&explicit_constructor, "sample_core", &config);
    assert!(constructor_out.contains("Self::new("), "{constructor_out}");
    assert!(
        constructor_out.contains("serde_wasm_bindgen::from_value(val.authentication.clone())"),
        "{constructor_out}"
    );

    for output in [
        &ordinary_out,
        &boxed_out,
        &private_out,
        &default_seeded_out,
        &constructor_out,
    ] {
        assert_strict_wasm_decode(output);
    }
}

pub(super) fn assert_strict_wasm_decode(output: &str) {
    assert!(output.contains("wasm_bindgen::throw_val"), "{output}");
    assert!(
        !output.contains("from_value(val.authentication.clone()).unwrap_or_default()"),
        "{output}"
    );
}

pub(super) fn lifetime_authentication_owner() -> TypeDef {
    let mut typ = type_with_field(required_authentication_field(false));
    typ.has_default = false;
    typ.has_lifetime_params = true;
    typ.methods.push(MethodDef {
        name: "with_owned".to_string(),
        params: vec![ParamDef {
            name: "authentication".to_string(),
            ty: TypeRef::Named("Authentication".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Named("ProcessConfig".to_string()),
        is_static: true,
        ..Default::default()
    });
    typ
}

#[test]
fn wasm_lifetime_constructor_required_tagged_field_uses_throwing_decode() {
    let tagged_names: AHashSet<String> = ["Authentication".to_string()].into_iter().collect();
    let config = wasm_required_tagged_config(&tagged_names);
    let output = gen_from_lifetime_type_constructor(
        &lifetime_authentication_owner(),
        "crate::ProcessConfig",
        "WasmProcessConfig",
        "crate",
        &config,
    )
    .expect("lifetime constructor conversion");

    assert!(
        output.contains("serde_wasm_bindgen::from_value(val.authentication.clone())"),
        "{output}"
    );
    assert!(output.contains("wasm_bindgen::throw_val"), "{output}");
    assert!(!output.contains("val.authentication.into()"), "{output}");
    assert!(!output.contains("unwrap_or_default"), "{output}");

    let non_wasm_output = gen_from_lifetime_type_constructor(
        &lifetime_authentication_owner(),
        "crate::ProcessConfig",
        "ProcessConfigBinding",
        "crate",
        &ConversionConfig::default(),
    )
    .expect("non-WASM lifetime constructor conversion");
    assert!(
        non_wasm_output.contains("val.authentication.into()"),
        "{non_wasm_output}"
    );
    assert!(
        !non_wasm_output.contains("wasm_bindgen::throw_val"),
        "{non_wasm_output}"
    );
}

#[test]
fn wasm_lifetime_constructor_recased_required_tagged_field_keeps_fallible_pipeline() {
    let tagged_names: AHashSet<String> = ["Authentication".to_string()].into_iter().collect();
    let recased = HashMap::from([(
        "Authentication".to_string(),
        WasmCamelRecasedEnum {
            out_wire_type: "__AlefWireOutWasmAuthentication",
            in_wire_type: "__AlefWireInWasmAuthentication",
            retag_fn_name: "__alef_wire_retag_wasm",
            core_tag_key: "auth_type",
            js_tag_key: "authType",
        },
    )]);
    let config = ConversionConfig {
        wasm_camel_recased_enums: Some(&recased),
        ..wasm_required_tagged_config(&tagged_names)
    };
    let output = gen_from_lifetime_type_constructor(
        &lifetime_authentication_owner(),
        "crate::ProcessConfig",
        "WasmProcessConfig",
        "crate",
        &config,
    )
    .expect("lifetime constructor conversion");

    assert!(output.contains("__AlefWireInWasmAuthentication"), "{output}");
    assert!(output.contains("__alef_wire_retag_wasm"), "{output}");
    assert_eq!(output.matches("unwrap_or_else(|error|").count(), 4, "{output}");
    assert!(!output.contains("val.authentication.into()"), "{output}");
    assert!(!output.contains("unwrap_or_default"), "{output}");
    assert!(!output.contains(".ok()"), "{output}");
}

#[test]
fn bare_field_default_on_non_default_owner_uses_field_default_only() {
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        rust_path: "sample_core::CaptioningConfig".to_string(),
        fields: vec![
            FieldDef {
                name: "llm".to_string(),
                ty: TypeRef::Named("LlmConfig".to_string()),
                ..Default::default()
            },
            FieldDef {
                name: "alt_text".to_string(),
                ty: TypeRef::Named("CaptionAltTextMode".to_string()),
                default: Some("/* serde(default) */".to_string()),
                typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        optionalize_bare_field_defaults: true,
        ..Default::default()
    };

    let out = gen_from_binding_to_core_cfg(&typ, "sample_core", &config);

    assert!(out.contains("llm: val.llm.into()"), "{out}");
    assert!(
        out.contains("alt_text: val.alt_text.map(|__v| __v.into()).unwrap_or_default()"),
        "{out}"
    );
    assert!(!out.contains("let mut __result"), "{out}");
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
        sensitive: false,
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
        sensitive: false,
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
        sensitive: false,
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
        sensitive: false,
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
        sensitive: false,
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
        sensitive: false,
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

pub(super) fn string_field(name: &str) -> FieldDef {
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
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}
