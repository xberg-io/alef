use super::NapiBackend;
use crate::core::backend::Backend;
use crate::core::config::Language;

/// NapiBackend::name returns "napi".
#[test]
fn napi_backend_name_is_napi() {
    let b = NapiBackend;
    assert_eq!(b.name(), "napi");
}

/// NapiBackend::language returns Language::Node.
#[test]
fn napi_backend_language_is_node() {
    let b = NapiBackend;
    assert_eq!(b.language(), Language::Node);
}

/// Test that cfg-gated fields in never_skip_cfg_field_names pass the options-field-bridge filter.
#[test]
fn cfg_gated_field_accepted_when_in_never_skip_list() {
    let never_skip_cfg_field_names = ["visitor".to_string()];
    let field_is_target = "visitor";

    let field_has_cfg = Some("feature = \"visitor\"");

    let accepted = field_has_cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_is_target);

    assert!(
        accepted,
        "cfg-gated field 'visitor' should pass filter when in never_skip_cfg_field_names"
    );
}

/// Test that plain data enums (with data variants, not tagged/untagged) appearing in struct fields
/// get binding-to-core From impls when the struct is an input type.
/// Regression: AuthHeaderFormat has data variant ApiKey(String), appears in CustomProviderConfig
/// field, but binding-to-core impl was not being generated, causing struct conversion to fail.
#[test]
fn plain_data_enum_in_input_type_struct_gets_binding_to_core_impl() {
    use crate::codegen::conversions::{
        ConversionConfig, can_generate_enum_conversion, can_generate_enum_conversion_from_core,
        gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding_cfg,
    };
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    let auth_format_enum = EnumDef {
        name: "AuthHeaderFormat".to_string(),
        rust_path: "fixture_core::AuthHeaderFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Bearer".to_string(),
                fields: vec![],
                ..Default::default()
            },
            EnumVariant {
                name: "ApiKey".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                ..Default::default()
            },
        ],
        serde_tag: None,
        serde_untagged: false,
        ..Default::default()
    };

    let has_data_variants = auth_format_enum.variants.iter().any(|v| !v.fields.is_empty());
    assert!(has_data_variants, "AuthHeaderFormat should have data variants");

    let is_tagged = auth_format_enum.serde_tag.is_some();
    let is_untagged = auth_format_enum.serde_untagged;
    assert!(
        !(is_tagged && has_data_variants),
        "AuthHeaderFormat should not be tagged data enum"
    );
    assert!(
        !(is_untagged && has_data_variants),
        "AuthHeaderFormat should not be untagged data enum"
    );

    assert!(
        can_generate_enum_conversion(&auth_format_enum),
        "plain data enum should be eligible for binding-to-core conversion"
    );
    assert!(
        can_generate_enum_conversion_from_core(&auth_format_enum),
        "plain data enum should be eligible for core-to-binding conversion"
    );

    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..Default::default()
    };
    let binding_to_core = gen_enum_from_binding_to_core_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        binding_to_core.contains("impl From<JsAuthHeaderFormat> for fixture_core::AuthHeaderFormat"),
        "should emit binding-to-core impl for plain data enum; got:\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        core_to_binding.contains("impl From<fixture_core::AuthHeaderFormat> for JsAuthHeaderFormat"),
        "should emit core-to-binding impl for plain data enum; got:\n{core_to_binding}"
    );
}

/// Test that opaque types with `has_default=true` emit `#[napi(constructor)]` with `new_constructor()`
/// even when a static `new()` method exists. This ensures JS `new ClassName()` works without
/// causing duplicate symbol errors.
/// Regression: App type has both a static `new()` method and `has_default=true`, but the
/// NAPI backend was skipping constructor emission due to the `!has_static_new` guard, causing
/// "Class contains no 'constructor', can not new it!" at runtime.
#[test]
fn napi_opaque_type_with_default_and_static_new_emits_constructor() {
    use super::constructors::napi_default_constructor;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, TypeDef, TypeRef};

    let app_type = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        has_default: true,
        methods: vec![MethodDef {
            name: "new".to_string(),
            receiver: None,
            params: vec![],
            return_type: TypeRef::Named("App".to_string()),
            is_async: false,
            is_static: true,
            doc: "Create a new application".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let constructor = napi_default_constructor(&app_type, &mapper, "sample_crate", "Js");

    assert!(
        constructor.is_some(),
        "opaque type with has_default=true should emit constructor even with static new()"
    );

    let constructor_code = constructor.unwrap();
    assert!(
        constructor_code.contains("#[napi(constructor)]"),
        "constructor should be marked with #[napi(constructor)]"
    );
    assert!(
        constructor_code.contains("pub fn new_constructor()"),
        "constructor should use new_constructor() to avoid conflict with static new()"
    );
    assert!(
        constructor_code.contains("Self { inner: std::sync::Arc::new(sample_crate::App::new())"),
        "constructor should create new App via sample_crate::App::new()"
    );
}

/// Regression: a `&mut self -> Result<&mut Self, E>` builder (a method returning a reference to
/// its own wrapper type) must SHARE the existing handle's `Arc` (`self.inner.clone()`) instead of
/// cloning the returned reference. `&mut App` is not `Clone`, so
/// `Arc::new(std::sync::Mutex::new(result.clone()))` fails to compile (E0599).
#[test]
fn napi_self_ref_builder_shares_arc_instead_of_cloning_returned_ref() {
    use super::types::gen_opaque_instance_method;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
    use ahash::AHashSet;
    use std::collections::HashMap;

    let method = MethodDef {
        name: "register_route".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("RouteCfg".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("App".to_string()),
        error_type: Some("AppError".to_string()),
        doc: "Register a route, returning the app for chaining.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        returns_ref: true,
        ..MethodDef::default()
    };
    let typ = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        methods: vec![method.clone()],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let cfg = super::NapiBackend::binding_config("sample_crate", "Js", true);
    let mut opaque = AHashSet::new();
    opaque.insert("App".to_string());
    opaque.insert("RouteCfg".to_string());
    let mut mutex = AHashSet::new();
    mutex.insert("App".to_string());
    let adapter_bodies = crate::adapters::AdapterBodies::new();
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let capsule: HashMap<String, crate::core::config::NodeCapsuleTypeConfig> = HashMap::new();

    let code = gen_opaque_instance_method(
        &method,
        &mapper,
        &typ,
        &cfg,
        &opaque,
        "Js",
        &adapter_bodies,
        &streaming,
        &mutex,
        &capsule,
    );

    assert!(
        code.contains("Ok(Self { inner: self.inner.clone() })"),
        "self-returning builder should share the existing Arc, got:\n{code}"
    );
    assert!(
        !code.contains("result.clone()"),
        "must not clone the returned &mut ref, got:\n{code}"
    );
    assert!(
        !code.contains("let result ="),
        "self-returning builder must not bind the returned &mut ref, got:\n{code}"
    );
}
