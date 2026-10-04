use super::{gen_dto_method_fns, gen_opaque_instance_method, gen_static_method, gen_struct, napi_field_is_optional};
use crate::backends::napi::type_map::NapiMapper;
use crate::core::ir::{
    FieldDef, MethodDef, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, ReceiverKind, SerdeContainerConversion,
    TypeDef, TypeRef,
};

fn transparent_string_wrapper() -> String {
    NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "sample_core::SecretString",
        "from",
        "into_inner",
        Vec::new(),
    )])
}

fn secret_method(name: &str) -> MethodDef {
    let wrapper = transparent_string_wrapper();
    MethodDef {
        name: name.to_owned(),
        params: vec![ParamDef {
            name: "value".to_owned(),
            ty: TypeRef::String,
            newtype_wrapper: Some(wrapper.clone()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::String,
        return_newtype_wrapper: Some(wrapper),
        ..MethodDef::default()
    }
}

fn secret_owner(method: MethodDef, is_opaque: bool) -> TypeDef {
    TypeDef {
        name: "SecretStore".to_owned(),
        rust_path: "sample_core::SecretStore".to_owned(),
        methods: vec![method],
        is_opaque,
        ..TypeDef::default()
    }
}

fn napi_binding_config() -> crate::codegen::generators::RustBindingConfig<'static> {
    use crate::backends::napi::gen_bindings::NapiBackend;

    NapiBackend::binding_config("sample_core", "Js", true)
}

/// gen_struct (pub(super)) is accessible from mod.rs — smoke test via trait.
/// The actual output is tested via the integration test (gen_bindings_test.rs).
#[test]
fn struct_gen_function_exists() {}

#[test]
fn bare_serde_default_is_optional_without_optionalizing_required_sibling() {
    let llm = FieldDef {
        name: "llm".to_string(),
        ty: TypeRef::Named("LlmConfig".to_string()),
        ..Default::default()
    };
    let alt_text = FieldDef {
        name: "alt_text".to_string(),
        ty: TypeRef::Named("CaptionAltTextMode".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string())),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        fields: vec![llm.clone(), alt_text.clone()],
        ..Default::default()
    };

    assert!(!napi_field_is_optional(&llm, &typ));
    assert!(napi_field_is_optional(&alt_text, &typ));

    let rendered = gen_struct(
        &typ,
        &NapiMapper::new("Js".to_string()),
        "Js",
        true,
        &ahash::AHashSet::default(),
        &[],
        &[],
        "sample_core",
        &ahash::AHashSet::default(),
        None,
        &[],
    );
    assert!(rendered.contains("pub llm: JsLlmConfig"), "{rendered}");
    assert!(
        rendered.contains("pub alt_text: Option<JsCaptionAltTextMode>"),
        "{rendered}"
    );
}

#[test]
fn non_enum_bare_default_preserves_required_field_shape() {
    let field = FieldDef {
        name: "quality_thresholds".to_string(),
        ty: TypeRef::Named("OcrQualityThresholds".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(crate::core::ir::DefaultValue::Empty),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "OcrPipelineConfig".to_string(),
        fields: vec![field.clone()],
        ..Default::default()
    };

    assert!(!napi_field_is_optional(&field, &typ));
}

/// A field's `#[napi(js_name = ...)]` must come from casing policy alone, never from
/// `#[serde(rename = ...)]` on the core struct -- the two are separate name surfaces (the
/// public JS identifier vs. the JSON wire key), and `gen_dts` (this backend's `.d.ts`
/// generator) already computes the JS-visible name from casing policy only. Before this
/// fix, a field with an explicit `serde_rename` made the *compiled* binding expose that
/// wire name in JS while the generated `.d.ts` kept the camelCase name for the same field
/// from the same IR -- the artifact and its own tracked declaration disagreed.
#[test]
fn js_name_ignores_serde_rename_but_wire_rename_is_preserved() {
    let typ = TypeDef {
        name: "ChunkerConfig".to_string(),
        fields: vec![FieldDef {
            name: "max_characters".to_string(),
            serde_rename: Some("max_chars".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &ahash::AHashSet::default(),
        None,
        &[],
    );

    assert!(
        out.contains("js_name = \"maxCharacters\""),
        "js_name must use casing policy (maxCharacters), matching gen_dts's .d.ts output:\n{out}"
    );
    assert!(
        !out.contains("js_name = \"max_chars\""),
        "js_name must not bleed the wire (serde) rename into the public JS identifier:\n{out}"
    );
    assert!(
        out.contains("serde(rename = \"max_chars\")"),
        "the field's own wire rename must still reach #[serde(rename = ...)] independently:\n{out}"
    );
}

fn f64_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
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

/// Extracts the `#[derive(...)]` line so assertions on delegation don't get fooled by
/// "serde::Deserialize" also appearing inside the delegating impl body text.
fn derive_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("rendered struct has a derive line")
}

#[test]
fn delegates_deserialize_for_sound_two_field_pair_in_convertible_set() {
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["Point".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
        &[],
    );

    assert!(
        !derive_line(&out).contains("serde::Deserialize"),
        "derive line must drop Deserialize when delegating: {out}"
    );
    assert!(
        out.contains("impl<'de> serde::Deserialize<'de> for JsPoint {"),
        "expected a delegating Deserialize impl in: {out}"
    );
    assert!(
        out.contains("<sample_core::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegating impl must read the core type: {out}"
    );
}

#[test]
fn keeps_derive_when_type_not_confirmed_in_convertible_set() {
    // Sound fields and a real container conversion, but the caller never proved a matching
    // `From<core::Type>` impl will exist for this run (empty convertible set) -- must NOT
    // delegate, since `.into()` would call a `From` impl that might not be emitted.
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &ahash::AHashSet::default(),
        None,
        &[],
    );

    assert!(derive_line(&out).contains("serde::Deserialize"));
    assert!(!out.contains("impl<'de> serde::Deserialize<'de> for JsPoint"));
}

#[test]
fn keeps_derive_when_unsound_opaque_field() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        rust_path: "sample_core::Wrapper".to_string(),
        fields: vec![FieldDef {
            name: "handle".to_string(),
            ty: TypeRef::Named("OpaqueHandle".to_string()),
            ..Default::default()
        }],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types: ahash::AHashSet<String> = ["OpaqueHandle".to_string()].into_iter().collect();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["Wrapper".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
        &[],
    );

    // Falls back to the derived, field-by-field Deserialize -- the existing
    // SerdeContainerConversionUnsupported diagnostic keeps naming the real gap here.
    assert!(derive_line(&out).contains("serde::Deserialize"));
    assert!(!out.contains("impl<'de> serde::Deserialize<'de> for JsWrapper"));
}

/// The node mirror drops every per-field `#[serde(default)]` the core struct carries, so
/// `JSON.parse`-shaped partial objects that the core type accepts are rejected at the binding.
/// Asserted on the rendered struct, not on the predicate.
#[test]
fn delegates_deserialize_for_field_with_serde_default() {
    let typ = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "sample_core::ExtractionConfig".to_string(),
        fields: vec![
            f64_field("threshold"),
            FieldDef {
                default: Some("/* serde(default) */".to_string()),
                ..f64_field("timeout")
            },
        ],
        is_opaque: false,
        has_serde: true,
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["ExtractionConfig".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &ahash::AHashSet::default(),
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
        &[],
    );

    assert!(
        !derive_line(&out).contains("serde::Deserialize"),
        "a derived Deserialize drops the field's serde(default): {out}"
    );
    assert!(
        out.contains("impl<'de> serde::Deserialize<'de> for JsExtractionConfig {"),
        "expected a delegating Deserialize impl in: {out}"
    );
    assert!(
        out.contains(
            "<sample_core::ExtractionConfig as serde::Deserialize>::deserialize(deserializer).map(Into::into)"
        ),
        "delegation must read the core type: {out}"
    );
}

/// Positive control: nothing about this struct disagrees with the derived, field-by-field
/// `Deserialize`, so it must keep deriving and must NOT gain a delegating impl.
#[test]
fn keeps_derive_for_struct_with_no_unreproducible_serde_attrs() {
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["Point".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &ahash::AHashSet::default(),
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
        &[],
    );

    assert!(derive_line(&out).contains("serde::Deserialize"), "{out}");
    assert!(!out.contains("impl<'de> serde::Deserialize<'de> for JsPoint"), "{out}");
}

/// The napi prelude puts its own one-parameter `Result<T> = Result<T, napi::Error>` alias in
/// scope in every generated node binding, so an unqualified `Result<Self, D::Error>` in the
/// delegating impl resolves to `napi::Result` and the signature stops matching
/// `serde::Deserialize`. This produced 174 compile errors (58 delegating impls x E0053/E0277/
/// E0308) in a real consumer the first time this backend emitted the impl at all. The return
/// type must be spelled absolutely. ~keep
#[test]
fn delegating_deserialize_return_type_is_immune_to_the_napi_result_alias() {
    let typ = TypeDef {
        name: "ChatRequest".to_string(),
        rust_path: "sample_core::ChatRequest".to_string(),
        fields: vec![FieldDef {
            default: Some("/* serde(default) */".to_string()),
            ..f64_field("temperature")
        }],
        is_opaque: false,
        has_serde: true,
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["ChatRequest".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &ahash::AHashSet::default(),
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
        &[],
    );

    assert!(
        out.contains("fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>"),
        "the delegating impl's return type must not be resolvable to napi's `Result` alias: {out}"
    );
}

#[test]
fn napi_instance_transparent_string_method_converts_both_boundaries() {
    let mut method = secret_method("replace_secret");
    method.receiver = Some(ReceiverKind::RefMut);
    let owner = secret_owner(method.clone(), true);
    let mapper = NapiMapper::new("Js".to_owned());
    let opaque_types = [owner.name.clone()].into_iter().collect();

    let output = gen_opaque_instance_method(
        &method,
        &mapper,
        &owner,
        &napi_binding_config(),
        &opaque_types,
        "Js",
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );

    assert!(
        output.contains("replace_secret(sample_core::SecretString::from(value))"),
        "instance input must construct the wrapper:\n{output}"
    );
    assert!(
        output.contains("into_inner()"),
        "instance return must consume the wrapper:\n{output}"
    );
}

#[test]
fn napi_static_async_and_fallible_transparent_string_methods_convert_both_boundaries() {
    let mapper = NapiMapper::new("Js".to_owned());
    let opaque_types = ahash::AHashSet::default();
    let mutex_types = ahash::AHashSet::default();

    for (name, is_async, error_type) in [
        ("replace_async", true, None),
        ("replace_fallible", false, Some("sample_core::Error".to_owned())),
    ] {
        let mut method = secret_method(name);
        method.is_static = true;
        method.is_async = is_async;
        method.error_type = error_type;
        method.params[0].is_ref = name == "replace_fallible";
        let owner = secret_owner(method.clone(), true);
        let output = gen_static_method(
            &method,
            &mapper,
            &owner,
            &napi_binding_config(),
            &opaque_types,
            "Js",
            &mutex_types,
        );

        assert!(
            output.contains("sample_core::SecretString::from(value)"),
            "{name} input must construct the wrapper:\n{output}"
        );
        assert!(
            output.contains("into_inner()"),
            "{name} return must consume the wrapper:\n{output}"
        );
        if method.params[0].is_ref {
            let declaration = output.find("let value_newtype").expect("wrapper local declaration");
            let call = output
                .find("replace_fallible(&value_newtype)")
                .expect("wrapper local borrow");
            assert!(
                declaration < call,
                "static wrapper local must be declared before it is borrowed:\n{output}"
            );
        }
    }
}

#[test]
fn napi_dto_instance_wither_declares_borrowed_wrapper_local_before_use() {
    let mut method = secret_method("with_secret");
    method.receiver = Some(ReceiverKind::Ref);
    method.params[0].is_ref = true;
    method.return_type = TypeRef::Named("SecretStore".to_owned());
    method.return_newtype_wrapper = None;
    let owner = secret_owner(method, false);
    let output = gen_dto_method_fns(
        &owner,
        &NapiMapper::new("Js".to_owned()),
        &napi_binding_config(),
        &ahash::AHashSet::default(),
        "Js",
        &ahash::AHashSet::default(),
        &crate::core::ir::ApiSurface::default(),
    );

    let declaration = output.find("let value_newtype").expect("wrapper local declaration");
    let call = output
        .find("with_secret(&value_newtype)")
        .expect("wrapper local borrow");
    assert!(
        declaration < call,
        "wrapper local must be declared before the wither borrows it:\n{output}"
    );
}
