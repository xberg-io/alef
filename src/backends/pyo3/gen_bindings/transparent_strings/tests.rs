use super::*;
use crate::backends::pyo3::gen_bindings::Pyo3Backend;
use crate::backends::pyo3::type_map::Pyo3Mapper;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, EnumVariant, NewtypeWrapperMetadata, PrimitiveType};

pub(super) fn wrapper(containers: Vec<NewtypeContainer>) -> String {
    wrapper_with_operations(containers, "from", "into_inner")
}

pub(super) fn fixture_wrapper(containers: Vec<NewtypeContainer>) -> String {
    NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "fixture_core::SecretString",
        "from",
        "into_inner",
        containers,
    )])
}

pub(super) fn wrapper_with_operations(containers: Vec<NewtypeContainer>, from: &str, into: &str) -> String {
    NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "toolkit::SecretString",
        from,
        into,
        containers,
    )])
}

pub(super) fn wrappers(paths: &[Vec<NewtypeContainer>]) -> String {
    NewtypeWrapper::encode_explicit(
        &paths
            .iter()
            .map(|containers| {
                NewtypeWrapperMetadata::transparent_string(
                    "toolkit::SecretString",
                    "from",
                    "into_inner",
                    containers.clone(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn adapter(typ: &TypeDef, method: &MethodDef) -> Option<String> {
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("toolkit", false);
    method_adapter(
        typ,
        method,
        "toolkit",
        &ahash::AHashSet::new(),
        &Pyo3Mapper::default(),
        &cfg,
    )
}

pub(super) fn optional_credential_constructor_owner(
    name: &str,
    constructor: &str,
    has_lifetime_params: bool,
) -> TypeDef {
    let field = FieldDef {
        name: "optional_credential".to_string(),
        ty: TypeRef::String,
        optional: true,
        newtype_wrapper: Some(fixture_wrapper(vec![NewtypeContainer::Optional])),
        ..FieldDef::default()
    };
    TypeDef {
        name: name.to_string(),
        rust_path: format!("fixture_core::{name}"),
        fields: vec![field.clone()],
        methods: vec![MethodDef {
            name: constructor.to_string(),
            params: vec![ParamDef {
                name: field.name,
                ty: field.ty,
                optional: field.optional,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named(name.to_string()),
            is_static: true,
            ..MethodDef::default()
        }],
        has_lifetime_params,
        ..TypeDef::default()
    }
}

pub(super) fn gate_fixture_config() -> crate::core::config::ResolvedCrateConfig {
    let config: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.python]
module_name = "toolkit"
"#,
    )
    .expect("fixture config must parse");
    config.resolve().expect("fixture config must resolve").remove(0)
}

pub(super) fn optional_map_method() -> (TypeDef, MethodDef) {
    let method = MethodDef {
        name: "optional_secret_map".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            is_ref: true,
            newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue])),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };
    let typ = TypeDef {
        name: "Segment".to_string(),
        rust_path: "toolkit::Segment".to_string(),
        methods: vec![method.clone()],
        ..TypeDef::default()
    };
    (typ, method)
}

#[test]
fn optional_map_param_borrows_after_explicit_wrapper_conversion() {
    let (_, method) = optional_map_method();
    assert_eq!(
            converted_param(&method.params[0], false, 0),
            Some((
                "let __alef_wrapper_arg_0 = (value).map(|value| (value).into_iter().map(|(key, value)| (key, toolkit::SecretString::from(value))).collect());\n        ".to_string(),
                "__alef_wrapper_arg_0.as_ref()".to_string()
            ))
        );
}

#[test]
fn gate_fixture_root_optional_enum_and_function_paths_use_constructor_items() {
    let optional_wrapper = wrapper(vec![NewtypeContainer::Optional]);
    let optional_param = |is_ref| ParamDef {
        name: "value".to_string(),
        ty: TypeRef::String,
        optional: true,
        is_ref,
        newtype_wrapper: Some(optional_wrapper.clone()),
        ..ParamDef::default()
    };
    let api = ApiSurface {
        crate_name: "toolkit".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "Authentication".to_string(),
            rust_path: "toolkit::Authentication".to_string(),
            variants: vec![EnumVariant {
                name: "Optional".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    newtype_wrapper: Some(optional_wrapper.clone()),
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            }],
            has_serde: true,
            ..EnumDef::default()
        }],
        functions: vec![
            FunctionDef {
                name: "optional_secret".to_string(),
                rust_path: "toolkit::optional_secret".to_string(),
                params: vec![optional_param(false)],
                return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                return_newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "borrow_optional_secret".to_string(),
                rust_path: "toolkit::borrow_optional_secret".to_string(),
                params: vec![optional_param(true)],
                return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                return_newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                ..FunctionDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let files = Pyo3Backend
        .generate_bindings(&api, &gate_fixture_config())
        .expect("gate fixture bindings must generate");
    let generated = &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .expect("PyO3 binding must emit lib.rs")
        .content;
    let redundant = "(value).map(|value| toolkit::SecretString::from(value))";
    let direct = "(value).map(toolkit::SecretString::from)";
    assert_eq!(generated.matches(redundant).count(), 0, "{generated}");
    assert_eq!(generated.matches(direct).count(), 3, "{generated}");
}

pub(super) fn cfg_optional_wrapper_enum(rust_path: &str) -> EnumDef {
    EnumDef {
        name: "Authentication".to_string(),
        rust_path: rust_path.to_string(),
        variants: vec![
            EnumVariant {
                name: "None".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Optional".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                    ..FieldDef::default()
                }],
                cfg: Some("feature = \"extras\"".to_string()),
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        ..EnumDef::default()
    }
}

pub(super) fn generated_lib_for_enum(enum_def: EnumDef) -> String {
    let api = ApiSurface {
        crate_name: "toolkit".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![enum_def],
        ..ApiSurface::default()
    };
    Pyo3Backend
        .generate_bindings(&api, &gate_fixture_config())
        .expect("cfg enum bindings must generate")
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .expect("PyO3 binding must emit lib.rs")
        .content
}

#[test]
fn foreign_cfg_variant_is_excluded_from_constructor_rewrite_eligibility() {
    let generated = generated_lib_for_enum(cfg_optional_wrapper_enum("foreign_core::Authentication"));

    assert!(!generated.contains("fn _factory_optional"), "{generated}");
    assert!(
        !generated.contains("(value).map(toolkit::SecretString::from)"),
        "{generated}"
    );
}

#[test]
fn host_cfg_variant_remains_eligible_for_constructor_rewrite() {
    let generated = generated_lib_for_enum(cfg_optional_wrapper_enum("toolkit::Authentication"));

    assert!(
        generated.contains("#[cfg(feature = \"extras\")]\n    #[pyo3(name = \"optional\")]"),
        "{generated}"
    );
    assert!(generated.contains("fn _factory_optional"), "{generated}");
    assert!(
        generated.contains("(value).map(toolkit::SecretString::from)"),
        "{generated}"
    );
    assert!(
        !generated.contains("map(|value| toolkit::SecretString::from(value))"),
        "{generated}"
    );
}

#[test]
fn single_optional_custom_constructor_specializes_ordinary_field() {
    let field = FieldDef {
        name: "credential".to_string(),
        ty: TypeRef::String,
        optional: true,
        newtype_wrapper: Some(wrapper_with_operations(
            vec![NewtypeContainer::Optional],
            "new_secret",
            "expose_secret",
        )),
        ..FieldDef::default()
    };

    assert_eq!(
        binding_to_core_field_expr(&field, "credential"),
        Some("(val.credential).map(toolkit::SecretString::new_secret)".to_string())
    );

    let typ = TypeDef {
        name: "Config".to_string(),
        rust_path: "toolkit::Config".to_string(),
        fields: vec![field],
        ..TypeDef::default()
    };
    let config = crate::codegen::conversions::ConversionConfig::default();
    let shared = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config);
    assert!(
        shared.contains("credential: (val.credential).map(|value| toolkit::SecretString::new_secret(value)),"),
        "negative control must contain the redundant closure: {shared}"
    );
    let rewritten = rewrite_binding_to_core_fields(shared, &typ, &config);
    assert!(
        rewritten.contains("credential: (val.credential).map(toolkit::SecretString::new_secret),"),
        "{rewritten}"
    );
    assert!(!rewritten.contains(".map(|value|"), "{rewritten}");
}

#[test]
fn report_optional_credential_rewrite_ignores_nested_field_suffix_match() {
    let typ = TypeDef {
        name: "Report".to_string(),
        rust_path: "fixture_core::Report".to_string(),
        fields: vec![
            FieldDef {
                name: "optional_credential".to_string(),
                ty: TypeRef::String,
                optional: true,
                newtype_wrapper: Some(fixture_wrapper(vec![NewtypeContainer::Optional])),
                ..FieldDef::default()
            },
            FieldDef {
                name: "nested_optional_credential".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                newtype_wrapper: Some(fixture_wrapper(vec![
                    NewtypeContainer::Optional,
                    NewtypeContainer::Optional,
                ])),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };
    let config = crate::codegen::conversions::ConversionConfig::default();
    let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
    let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);

    assert!(
        rewritten.contains("optional_credential: (val.optional_credential).map(fixture_core::SecretString::from),"),
        "{rewritten}"
    );
    assert!(
        rewritten.contains(concat!(
            "nested_optional_credential: ",
            "((val.nested_optional_credential).map(fixture_core::SecretString::from)).map(Some),"
        )),
        "{rewritten}"
    );
}

#[test]
fn static_new_owner_rewrites_exact_optional_credential_argument() {
    let typ = optional_credential_constructor_owner("Report", "new", false);
    let config = crate::codegen::conversions::ConversionConfig::default();
    let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
    assert!(generated.contains("Self::new("), "{generated}");
    assert!(generated.contains("val.optional_credential,"), "{generated}");

    let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);
    assert!(
        rewritten.contains("(val.optional_credential).map(fixture_core::SecretString::from),"),
        "{rewritten}"
    );
}

#[test]
fn lifetime_owner_rewrites_exact_optional_credential_argument() {
    let typ = optional_credential_constructor_owner("BorrowedReport", "with_owned", true);
    let config = crate::codegen::conversions::ConversionConfig::default();
    let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
    assert!(
        generated.contains("fixture_core::BorrowedReport::with_owned("),
        "{generated}"
    );
    assert!(generated.contains("val.optional_credential,"), "{generated}");

    let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);
    assert!(
        rewritten.contains("(val.optional_credential).map(fixture_core::SecretString::from),"),
        "{rewritten}"
    );
}

#[test]
fn single_optional_custom_constructor_specializes_owned_and_borrowed_params() {
    let mut param = ParamDef {
        name: "credential".to_string(),
        ty: TypeRef::String,
        optional: true,
        newtype_wrapper: Some(wrapper_with_operations(
            vec![NewtypeContainer::Optional],
            "new_secret",
            "expose_secret",
        )),
        ..ParamDef::default()
    };

    assert_eq!(
        converted_param(&param, false, 0),
        Some((
            String::new(),
            "(credential).map(toolkit::SecretString::new_secret)".to_string()
        ))
    );

    param.is_ref = true;
    assert_eq!(
        converted_param(&param, false, 0),
        Some((
            concat!(
                "let __alef_wrapper_arg_0 = ",
                "(credential).map(toolkit::SecretString::new_secret);\n        "
            )
            .to_string(),
            "__alef_wrapper_arg_0.as_ref()".to_string()
        ))
    );
}

#[test]
fn nested_optional_map_and_multipath_wrappers_keep_structural_conversion() {
    let nested_wrapper = wrapper_with_operations(
        vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
        "new_secret",
        "expose_secret",
    );
    let field = FieldDef {
        name: "credentials".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        optional: true,
        newtype_wrapper: Some(nested_wrapper.clone()),
        ..FieldDef::default()
    };
    assert_eq!(binding_to_core_field_expr(&field, "credentials"), None);

    let param = ParamDef {
        name: "credentials".to_string(),
        ty: field.ty.clone(),
        optional: true,
        newtype_wrapper: Some(nested_wrapper),
        ..ParamDef::default()
    };
    assert_eq!(
        converted_param(&param, false, 0),
        Some((
            String::new(),
            concat!(
                "(credentials).map(|value| (value).into_iter().map(|(key, value)| ",
                "(key, toolkit::SecretString::new_secret(value))).collect())"
            )
            .to_string()
        ))
    );

    let multipath = wrappers(&[
        vec![NewtypeContainer::Optional, NewtypeContainer::MapKey],
        vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
    ]);
    let multipath_param = ParamDef {
        name: "credentials".to_string(),
        ty: field.ty,
        optional: true,
        newtype_wrapper: Some(multipath),
        ..ParamDef::default()
    };
    assert_eq!(
        converted_param(&multipath_param, false, 0),
        Some((
            String::new(),
            concat!(
                "(credentials).map(|value| (value).into_iter().map(|(key, value)| ",
                "(toolkit::SecretString::from(key), toolkit::SecretString::from(value))).collect())"
            )
            .to_string()
        ))
    );
}

#[test]
fn named_leaf_hint_keeps_binding_hash_map_and_leaves_btree_target_collect_inferred() {
    struct BindingSegment;
    struct CoreSegment;
    impl From<BindingSegment> for CoreSegment {
        fn from(_: BindingSegment) -> Self {
            Self
        }
    }

    let ty = TypeRef::Optional(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("Segment".to_string())),
    )));
    assert_eq!(
        convert_named_leaves("values", &ty),
        concat!(
            "(values).map(|value: std::collections::HashMap<_, _>| ",
            "(value).into_iter().map(|(key, value)| (key, (value).into())).collect())"
        )
    );

    let values: Option<std::collections::HashMap<String, BindingSegment>> = None;
    let converted: Option<std::collections::BTreeMap<String, CoreSegment>> =
        values.map(|value: std::collections::HashMap<_, _>| {
            value.into_iter().map(|(key, value)| (key, value.into())).collect()
        });
    assert_eq!(converted.map(|value| value.len()), None);
}

#[test]
fn flattened_nested_optional_field_uses_explicit_wrapper_operations_in_both_directions() {
    let field = FieldDef {
        name: "credential".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        newtype_wrapper: Some(wrapper_with_operations(
            vec![NewtypeContainer::Optional, NewtypeContainer::Optional],
            "new_secret",
            "expose_secret",
        )),
        ..FieldDef::default()
    };

    let to_core = binding_to_core_field_expr(&field, "credential").expect("binding-to-core conversion");
    assert_eq!(
        to_core,
        "((val.credential).map(toolkit::SecretString::new_secret)).map(Some)"
    );

    let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
    assert!(from_core.contains("val.credential.flatten()"), "{from_core}");
    assert!(from_core.contains("expose_secret()"), "{from_core}");
    assert!(!from_core.contains("to_string()"), "{from_core}");
}

#[test]
fn flattened_fields_with_additional_core_wrappers_are_rejected_before_generation() {
    let base = FieldDef {
        name: "credential".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
        ..FieldDef::default()
    };

    let mut cases = Vec::new();
    let mut boxed = base.clone();
    boxed.is_boxed = true;
    cases.push(("boxed", boxed));
    for (name, core_wrapper) in [
        ("cow", CoreWrapper::Cow),
        ("box", CoreWrapper::Box),
        ("arc", CoreWrapper::Arc),
        ("arc_mutex", CoreWrapper::ArcMutex),
    ] {
        let mut field = base.clone();
        field.core_wrapper = core_wrapper;
        cases.push((name, field));
    }
    let mut vec_inner = base.clone();
    vec_inner.ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
    vec_inner.vec_inner_core_wrapper = CoreWrapper::Arc;
    vec_inner.newtype_wrapper = Some(wrapper(vec![
        NewtypeContainer::Optional,
        NewtypeContainer::Optional,
        NewtypeContainer::Vec,
    ]));
    cases.push(("vec_inner_arc", vec_inner));

    for (case_name, field) in cases {
        assert_flattened_wrapper_case_rejected(case_name, field);
    }

    assert_supported_flattened_wrapper_generates_both_directions(base);
}

pub(super) fn assert_flattened_wrapper_case_rejected(case_name: &str, field: FieldDef) {
    let api = ApiSurface {
        crate_name: "toolkit".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "toolkit::Config".to_string(),
            fields: vec![field],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let error = Pyo3Backend
        .generate_bindings(&api, &gate_fixture_config())
        .expect_err("unsupported flattened wrapper composition must fail before emission");
    let message = error.to_string();
    assert!(
        message.contains("PyO3 cannot generate flattened explicit-wrapper field Config.credential"),
        "{case_name}: {message}"
    );
}

pub(super) fn assert_supported_flattened_wrapper_generates_both_directions(base: FieldDef) {
    let supported_api = ApiSurface {
        crate_name: "toolkit".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "toolkit::Config".to_string(),
            fields: vec![base],
            ..TypeDef::default()
        }],
        functions: vec![FunctionDef {
            name: "consume_config".to_string(),
            rust_path: "toolkit::consume_config".to_string(),
            params: vec![ParamDef {
                name: "config".to_string(),
                ty: TypeRef::Named("Config".to_string()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let generated = Pyo3Backend
        .generate_bindings(&supported_api, &gate_fixture_config())
        .expect("flattened explicit wrapper without an additional core wrapper remains supported")
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .expect("PyO3 binding must emit lib.rs")
        .content;
    assert!(generated.contains("toolkit::SecretString::from"), "{generated}");
    assert!(generated.contains(".into_inner()"), "{generated}");
}

#[test]
fn opaque_type_flattened_wrapper_metadata_is_not_rejected() {
    let api = ApiSurface {
        crate_name: "toolkit".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueConfig".to_string(),
            rust_path: "toolkit::OpaqueConfig".to_string(),
            is_opaque: true,
            fields: vec![FieldDef {
                name: "credential".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                core_wrapper: CoreWrapper::Arc,
                newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    Pyo3Backend
        .generate_bindings(&api, &gate_fixture_config())
        .expect("opaque PyO3 handles do not emit or validate their source fields");
}

#[test]
fn flattened_nested_collection_converts_wrapper_keys_and_named_values() {
    let field = FieldDef {
        name: "segments".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("Segment".to_string())),
        ))))),
        optional: true,
        newtype_wrapper: Some(wrappers(&[vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
            NewtypeContainer::Vec,
            NewtypeContainer::MapKey,
        ]])),
        ..FieldDef::default()
    };

    let to_core = binding_to_core_field_expr(&field, "segments").expect("binding-to-core conversion");
    assert_eq!(
        to_core,
        concat!(
            "(((val.segments).map(|value| (value).into_iter().map(|value| ",
            "(value).into_iter().map(|(key, value)| ",
            "(toolkit::SecretString::from(key), value)).collect()).collect())).map(|value: ",
            "Vec<std::collections::HashMap<_, _>>| ",
            "(value).into_iter().map(|value| (value).into_iter().map(|(key, value)| ",
            "(key, (value).into())).collect()).collect())).map(Some)"
        )
    );

    let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
    assert!(from_core.contains("val.segments.flatten()"), "{from_core}");
    assert!(from_core.contains("(key).into_inner()"), "{from_core}");
    assert!(
        from_core.contains("|value: Vec<std::collections::HashMap<_, _>>|"),
        "{from_core}"
    );
    assert!(from_core.contains("(value).into()"), "{from_core}");
}
