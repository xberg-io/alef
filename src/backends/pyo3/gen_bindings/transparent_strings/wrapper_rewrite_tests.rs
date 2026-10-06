//! Field-conversion rewrite, default-seed and promoted-parameter coverage, split from tests.rs.

#[allow(unused_imports)]
use super::tests::*;
use super::*;

#[test]
fn field_conversion_replacement_requires_exactly_one_emitted_form() {
    let initializer = "        Self {\n            credential: stale,\n        }\n";
    let replaced = replace_field_conversion(initializer, "credential", "credential", "converted")
        .expect("one initializer must be replaced");
    assert!(replaced.contains("credential: converted,"), "{replaced}");
    assert!(!replaced.contains("credential: stale,"), "{replaced}");

    let assignment = "        let mut __result = Config::default();\n        __result.credential = stale;\n";
    let replaced = replace_field_conversion(assignment, "credential", "credential", "converted")
        .expect("one default-seeded assignment must be replaced");
    assert!(replaced.contains("__result.credential = converted;"), "{replaced}");
    assert!(!replaced.contains("__result.credential = stale;"), "{replaced}");

    assert_eq!(
        replace_field_conversion("Self { other: stale }\n", "credential", "credential", "converted"),
        None
    );
    assert_eq!(
        replace_field_conversion("__result.credential = stale", "credential", "credential", "converted"),
        None
    );
    assert_eq!(
        replace_field_conversion(
            "            credential: first,\n            credential: second,\n",
            "credential",
            "credential",
            "converted",
        ),
        None
    );
    assert_eq!(
        replace_field_conversion(
            "Self::new(val.credential, val.credential,)",
            "credential",
            "credential",
            "converted"
        ),
        None
    );
}

#[test]
fn field_conversion_replacement_respects_expression_boundaries() {
    let cases = [
        (
            "compact initializer",
            "Self { credential: stale, other: keep }",
            "Self { credential: converted, other: keep }",
        ),
        (
            "compact assignment",
            "let mut out = Config::default(); __result.credential = stale; __result.other = keep;",
            "let mut out = Config::default(); __result.credential = converted; __result.other = keep;",
        ),
        (
            "nested delimiters",
            "Self { credential: call((a, b), [c, d], Thing { x: 1, y: 2 }), other: keep }",
            "Self { credential: converted, other: keep }",
        ),
        (
            "quoted terminators",
            r#"Self { credential: call("a,;\"quoted\""), other: keep }"#,
            "Self { credential: converted, other: keep }",
        ),
        (
            "generic comma",
            "Self { credential: values.collect::<HashMap<_, _>>(), other: keep }",
            "Self { credential: converted, other: keep }",
        ),
        (
            "field-name suffix collision",
            "Self { credential: stale, nested_credential: keep }",
            "Self { credential: converted, nested_credential: keep }",
        ),
    ];

    for (case_name, input, expected) in cases {
        assert_eq!(
            replace_field_conversion(input, "credential", "credential", "converted").as_deref(),
            Some(expected),
            "{case_name}"
        );
    }
}

#[test]
fn non_flattened_optional_boxed_wrapper_keeps_shared_conversions() {
    let typ = TypeDef {
        name: "Config".to_string(),
        rust_path: "toolkit::Config".to_string(),
        fields: vec![FieldDef {
            name: "secret".to_string(),
            ty: TypeRef::String,
            optional: true,
            is_boxed: true,
            newtype_wrapper: Some(wrapper_with_operations(
                vec![NewtypeContainer::Optional],
                "new_secret",
                "expose_secret",
            )),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let config = crate::codegen::conversions::ConversionConfig::default();
    let shared_to_core = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config);
    assert_eq!(
        rewrite_binding_to_core_fields(shared_to_core.clone(), &typ, &config),
        shared_to_core
    );

    let shared_from_core =
        crate::codegen::conversions::gen_from_core_to_binding_cfg(&typ, "toolkit", &ahash::AHashSet::new(), &config);
    assert_eq!(
        rewrite_core_to_binding_fields(shared_from_core.clone(), &typ, &config),
        shared_from_core
    );
}

#[test]
fn field_rewrite_preserves_default_seed_and_binding_exclusions() {
    let typ = TypeDef {
        name: "Config".to_string(),
        rust_path: "toolkit::Config".to_string(),
        has_default: true,
        fields: vec![
            FieldDef {
                name: "credential".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                ..FieldDef::default()
            },
            FieldDef {
                name: "hidden".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                binding_excluded: true,
                newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                ..FieldDef::default()
            },
            FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Duration,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };
    let config = crate::codegen::conversions::ConversionConfig {
        option_duration_on_defaults: true,
        ..Default::default()
    };
    let to_core = rewrite_binding_to_core_fields(
        crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config),
        &typ,
        &config,
    );
    assert!(to_core.contains("__result.credential ="), "{to_core}");
    assert!(to_core.contains(".map(toolkit::SecretString::from)"), "{to_core}");
    assert!(
        to_core.contains("let mut __result = toolkit::Config::default()"),
        "{to_core}"
    );
    assert!(!to_core.contains("(value).map(|value|"), "{to_core}");
    assert!(!to_core.contains("hidden:"), "{to_core}");

    let from_core = rewrite_core_to_binding_fields(
        crate::codegen::conversions::gen_from_core_to_binding_cfg(&typ, "toolkit", &ahash::AHashSet::new(), &config),
        &typ,
        &config,
    );
    assert!(from_core.contains("credential:"), "{from_core}");
    assert!(from_core.contains("into_inner()"), "{from_core}");
    assert!(!from_core.contains("hidden:"), "{from_core}");
}

#[test]
fn scalar_wrapper_param_borrows_the_constructed_value() {
    let param = ParamDef {
        name: "value".to_string(),
        ty: TypeRef::String,
        is_ref: true,
        newtype_wrapper: Some(wrapper(vec![])),
        ..ParamDef::default()
    };
    assert_eq!(
        converted_param(&param, false, 0),
        Some((
            "let __alef_wrapper_arg_0 = toolkit::SecretString::from(value);\n        ".to_string(),
            "&__alef_wrapper_arg_0".to_string()
        ))
    );
}

#[test]
fn promoted_wrapper_param_unwraps_the_required_binding_value() {
    let params = vec![
        ParamDef {
            name: "optional".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..ParamDef::default()
        },
        ParamDef {
            name: "required".to_string(),
            ty: TypeRef::String,
            is_ref: true,
            newtype_wrapper: Some(wrapper(vec![])),
            ..ParamDef::default()
        },
    ];
    let (bindings, args) = converted_call_args(&params, &ahash::AHashSet::new());
    assert!(
        bindings.contains("SecretString::from(required.expect(\"'required' is required\"))"),
        "{bindings}"
    );
    assert_eq!(args, "optional, &__alef_wrapper_arg_1");
}

#[test]
fn adapter_preserves_plain_fallible_and_async_return_handling() {
    let (typ, method) = optional_map_method();
    let plain = adapter(&typ, &method).expect("plain adapter");
    assert!(plain.contains("let __alef_wrapper_arg_0"), "{plain}");
    assert!(plain.ends_with("__alef_wrapper_arg_0.as_ref())"), "{plain}");
    let mut fallible = method.clone();
    fallible.error_type = Some("toolkit::Error".to_string());
    let fallible = adapter(&typ, &fallible).expect("fallible adapter");
    assert!(fallible.contains("PyRuntimeError::new_err"), "{fallible}");
    assert!(fallible.ends_with("Ok(result)"), "{fallible}");
    let mut asynchronous = method;
    asynchronous.is_async = true;
    let asynchronous = adapter(&typ, &asynchronous).expect("async adapter");
    assert!(asynchronous.contains("future_into_py(py, async move"), "{asynchronous}");
    assert!(asynchronous.contains(".await;"), "{asynchronous}");
}

#[test]
fn adapter_unwraps_explicit_wrapper_returns() {
    let (typ, mut method) = optional_map_method();
    method.return_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    method.return_newtype_wrapper = Some(wrapper(vec![NewtypeContainer::MapValue]));
    let output = adapter(&typ, &method).expect("wrapper-return adapter");
    assert!(output.contains("(value).into_inner()"), "{output}");
}

#[test]
fn adapter_materializes_borrowed_wrapper_returns_before_unwrapping() {
    let (typ, mut method) = optional_map_method();
    method.return_type = TypeRef::String;
    method.return_newtype_wrapper = Some(wrapper(vec![]));
    method.returns_ref = true;
    let output = adapter(&typ, &method).expect("borrowed wrapper-return adapter");
    assert!(output.contains("(core_self.optional_secret_map("), "{output}");
    assert!(output.contains(").clone()).into_inner()"), "{output}");
    method.returns_ref = false;
    method.returns_cow = true;
    let output = adapter(&typ, &method).expect("cow wrapper-return adapter");
    assert!(output.contains(").into_owned()).into_inner()"), "{output}");
}

#[test]
fn adapter_uses_the_core_type_path_for_static_methods() {
    let (typ, mut method) = optional_map_method();
    method.is_static = true;
    method.receiver = None;
    let output = adapter(&typ, &method).expect("static adapter");
    assert!(output.contains("toolkit::Segment::optional_secret_map"), "{output}");
    assert!(!output.contains("let core_self"), "{output}");
}

#[test]
fn adapter_rejects_sanitized_methods_and_params() {
    let (typ, mut method) = optional_map_method();
    method.sanitized = true;
    assert_eq!(adapter(&typ, &method), None);
    method.sanitized = false;
    method.params[0].sanitized = true;
    assert_eq!(adapter(&typ, &method), None);
}

#[test]
fn adapter_preserves_non_trait_ref_mut_writeback_with_stable_wrapper_locals() {
    let (typ, mut method) = optional_map_method();
    method.receiver = Some(ReceiverKind::RefMut);
    method.params[0].is_mut = true;
    let output = adapter(&typ, &method).expect("functional ref-mut adapter");
    assert!(output.contains("let mut __alef_wrapper_arg_0"), "{output}");
    assert!(output.contains("__alef_wrapper_arg_0.as_mut()"), "{output}");
    assert!(output.ends_with("core_self.into()"), "{output}");
}
