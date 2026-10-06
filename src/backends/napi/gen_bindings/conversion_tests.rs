//! NAPI binding/core conversion coverage, split from tests.rs.

use super::NapiBackend;
use super::methods::gen_tagged_enum_core_to_binding;
use super::tests::*;
use crate::core::backend::Backend;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

/// Regression coverage for the `clippy::redundant_field_names` fix: an optional tagged-enum
/// field whose type needs no cast/wrap (e.g. plain `Option<String>`) falls into the fallback
/// match arm in `gen_tagged_enum_core_to_binding`, where the destructured core-side variable
/// is bound under the same name as the binding field it fills. The field-init expression must
/// use shorthand (`text`), never `text: text`.
#[test]
fn core_to_binding_uses_shorthand_for_self_assigned_optional_field() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        !core_to_binding.contains("text: text"),
        "self-assigned optional field must use shorthand, not `text: text`:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("text }") || core_to_binding.contains("text,"),
        "expected field-init shorthand for `text`:\n{core_to_binding}"
    );
}

/// Same fallback arm, but for a field whose type does need a cast/wrap (`Vec<Named>` maps
/// through `.into()`) — this must keep the real `field: expr` form, not collapse to shorthand.
#[test]
fn core_to_binding_keeps_expr_form_when_conversion_is_required() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Items".to_string(),
            fields: vec![FieldDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Widget".to_string()))),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        core_to_binding.contains("items: items.map(|v| v.into_iter().map(Into::into).collect())"),
        "Vec<Named> optional field must keep its real conversion expression:\n{core_to_binding}"
    );
}

/// Regression test: an optional `Named` tagged-enum field whose type has a generated binding
/// struct (`has_binding` in the old code) must NOT be dereferenced unless the CORE field is
/// actually `Option<Box<T>>`. Before the fix, `core_to_binding_field_init` picked the deref
/// form purely off `has_binding`, so a plain (unboxed) `Option<T>` field emitted
/// `nested.map(|v| (*v).into())`, which fails to compile with E0614 because `v` is `T`, not
/// `Box<T>`. Mirrors the real-world defect on `DocxAppProperties`/`CoreProperties`/`YearRange`
/// fields (all plain `Option<T>`, all with a binding struct, none boxed).
#[test]
fn core_to_binding_optional_unboxed_named_field_is_not_dereferenced() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "WithNested".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("Nested".to_string()),
                optional: true,
                is_boxed: false,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    // `struct_names` containing "Nested" is what makes `has_binding` true for this field --
    // exactly the condition the old code (wrongly) used to decide whether to dereference.
    let mut struct_names = AHashSet::new();
    struct_names.insert("Nested".to_string());
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        !core_to_binding.contains("nested.map(|v| (*v).into())"),
        "unboxed Option<T> field must not be dereferenced (E0614 -- v is T, not Box<T>):\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("nested: nested.map(|v| v.into())"),
        "expected the unboxed optional Named field conversion:\n{core_to_binding}"
    );
}

fn node_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Regression test for issue #380, exercised through the real `NapiBackend::generate_bindings`
/// path (not just the `gen_function` unit): a `&mut T` DTO parameter on a unit-returning free
/// function must render as a binding that returns the mutated intermediate.
#[test]
fn generate_bindings_writeback_free_function_returns_mutated_dto() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_record".to_string(),
            rust_path: "test_lib::tag_record".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = NapiBackend.generate_bindings(&api, &node_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("record_core.into()"),
        "expected the write-back tail through the real generate_bindings path:\n{}",
        lib.content
    );
}

/// `reject_unsupported_writeback` must fire through the real `NapiBackend::generate_bindings`
/// path: a `&mut T` DTO param on a function that ALSO returns a value has no free return slot
/// for the write-back value, so generation must fail loudly (naming the function).
#[test]
fn generate_bindings_rejects_mut_dto_param_with_non_unit_return() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_and_count".to_string(),
            rust_path: "test_lib::tag_and_count".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    };

    let error = NapiBackend
        .generate_bindings(&api, &node_config())
        .expect_err("a `&mut` DTO param plus a non-unit return must be rejected at generation time");
    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the offending function:\n{message}"
    );
}

#[test]
fn promoted_required_method_and_constructor_parameters_return_invalid_arg() {
    use super::constructors::napi_variant_wrapper_constructor;
    use super::types::{gen_opaque_instance_method, gen_static_method};
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
    use std::collections::HashMap;

    let params = vec![
        ParamDef {
            name: "label".into(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        },
        ParamDef {
            name: "limit".into(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        },
    ];
    let instance = MethodDef {
        name: "apply".into(),
        params: params.clone(),
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    let static_method = MethodDef {
        name: "build".into(),
        params: params.clone(),
        return_type: TypeRef::Named("Widget".into()),
        ..Default::default()
    };
    let constructor = MethodDef {
        name: "new".into(),
        params,
        return_type: TypeRef::Named("Widget".into()),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Widget".into(),
        rust_path: "sample_core::Widget".into(),
        is_opaque: true,
        is_variant_wrapper: true,
        methods: vec![instance.clone(), static_method.clone(), constructor],
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".into());
    let cfg = NapiBackend::binding_config("sample_core", "Js", true);
    let empty = AHashSet::new();
    let instance_output = gen_opaque_instance_method(
        &instance,
        &mapper,
        &typ,
        &cfg,
        &empty,
        "Js",
        &crate::adapters::AdapterBodies::new(),
        &ahash::AHashMap::new(),
        &empty,
        &HashMap::new(),
    );
    let static_output = gen_static_method(&static_method, &mapper, &typ, &cfg, &empty, "Js", &empty);
    let constructor_output = napi_variant_wrapper_constructor(&typ, &mapper, "sample_core", "Js").unwrap();

    for output in [&instance_output, &static_output, &constructor_output] {
        assert!(output.contains("label: Option<String>, limit: Option<u32>"), "{output}");
        assert!(output.contains("limit.ok_or_else"), "{output}");
        assert!(
            output.contains("-> Result") || output.contains("-> napi::Result"),
            "{output}"
        );
        assert!(!output.contains("expect("), "{output}");
    }
}

#[test]
fn flattened_optional_transparent_wrapper_uses_one_binding_option_layer() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "nested_optional_credential".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper_with_methods(
                vec![vec![NewtypeContainer::Optional, NewtypeContainer::Optional]],
                "new_secret",
                "expose_secret",
            )),
            ..Default::default()
        }],
        has_default: true,
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        cast_large_ints_to_i64: true,
        cast_f32_to_f64: true,
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    assert!(
        to_core.contains("__result.nested_optional_credential =")
            && to_core.contains("map(fixture_core::SecretString::new_secret)")
            && to_core.contains(".map(Some)"),
        "the single JS optional must convert once and reconstruct the outer Rust Option:\n{to_core}"
    );
    assert!(
        !to_core.contains("map(|value| fixture_core::SecretString::new_secret(value))"),
        "flattened optional wrapper must not emit Clippy's redundant-closure shape:\n{to_core}"
    );
    assert!(
        !to_core.contains("map(|value| (value).map("),
        "a String inside the flattened JS Option is not another iterator/Option:\n{to_core}"
    );

    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);
    assert!(
        from_core.contains("flatten()") && from_core.contains("expose_secret()"),
        "the two Rust options must flatten before consuming the wrapper:\n{from_core}"
    );
    assert!(
        !from_core.contains("nested_optional_credential.flatten().map(|v| v.to_string())"),
        "an explicit transparent wrapper must not require Display:\n{from_core}"
    );
}

#[test]
fn root_optional_transparent_wrapper_field_uses_configured_constructor_function() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "optional_credential".into(),
            ty: TypeRef::String,
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper_with_methods(
                vec![vec![NewtypeContainer::Optional]],
                "new_secret",
                "expose_secret",
            )),
            ..Default::default()
        }],
        ..Default::default()
    };

    let output = super::transparent_newtypes::gen_from_binding_to_core(
        &typ,
        "fixture_core",
        &ConversionConfig {
            type_name_prefix: "Js",
            ..Default::default()
        },
    );

    assert!(
        output.contains("optional_credential: (val.optional_credential).map(fixture_core::SecretString::new_secret)"),
        "root optional wrapper must use the configured constructor function:\n{output}"
    );
    assert!(
        !output.contains("map(|value| fixture_core::SecretString::new_secret(value))"),
        "root optional wrapper must not emit Clippy's redundant-closure shape:\n{output}"
    );
}

#[test]
fn constructor_owned_root_optional_wrapper_uses_configured_constructor_function() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, MethodDef, NewtypeContainer, ParamDef, TypeDef, TypeRef};

    for has_lifetime_params in [false, true] {
        let field = FieldDef {
            name: "optional_credential".into(),
            ty: TypeRef::String,
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper_with_methods(
                vec![vec![NewtypeContainer::Optional]],
                "new_secret",
                "expose_secret",
            )),
            ..Default::default()
        };
        let typ = TypeDef {
            name: "Report".into(),
            rust_path: "fixture_core::Report".into(),
            fields: vec![field],
            methods: vec![MethodDef {
                name: "new".into(),
                params: vec![ParamDef {
                    name: "optional_credential".into(),
                    ty: TypeRef::String,
                    optional: true,
                    ..Default::default()
                }],
                is_static: true,
                ..Default::default()
            }],
            has_lifetime_params,
            ..Default::default()
        };

        let output = super::transparent_newtypes::gen_from_binding_to_core(
            &typ,
            "fixture_core",
            &ConversionConfig {
                type_name_prefix: "Js",
                ..Default::default()
            },
        );

        assert!(output.contains("::new("), "constructor path must be emitted:\n{output}");
        assert!(
            output.contains("(val.optional_credential).map(fixture_core::SecretString::new_secret)"),
            "constructor argument must use the configured wrapper constructor:\n{output}"
        );
        assert!(
            !output.contains("map(|value| fixture_core::SecretString::new_secret(value))"),
            "constructor argument must not emit a redundant closure:\n{output}"
        );
    }
}

#[test]
fn constructor_owned_flattened_wrapper_preserves_nested_container_traversal() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, MethodDef, NewtypeContainer, ParamDef, TypeDef, TypeRef};

    let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::String),
    )))));
    let wrapper = transparent_string_wrapper(vec![
        vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
            NewtypeContainer::Vec,
            NewtypeContainer::MapKey,
        ],
        vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
            NewtypeContainer::Vec,
            NewtypeContainer::MapValue,
        ],
    ]);
    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "secrets".into(),
            ty: ty.clone(),
            optional: true,
            newtype_wrapper: Some(wrapper),
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "new".into(),
            params: vec![ParamDef {
                name: "secrets".into(),
                ty,
                optional: true,
                ..Default::default()
            }],
            is_static: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    let output = super::transparent_newtypes::gen_from_binding_to_core(
        &typ,
        "fixture_core",
        &ConversionConfig {
            type_name_prefix: "Js",
            ..Default::default()
        },
    );

    assert!(output.contains("::new("), "constructor path must be emitted:\n{output}");
    assert!(
        output.contains("fixture_core::SecretString::from(key)")
            && output.contains("fixture_core::SecretString::from(value)"),
        "nested map keys and values must still construct wrappers:\n{output}"
    );
    assert!(
        output.contains(".map(Some)"),
        "outer Rust option must be restored:\n{output}"
    );
}

#[test]
fn flattened_optional_wrapper_recurses_through_vec_map_and_named_sibling() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "nested_optional_segments".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Segment".into())),
            ))))),
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Optional,
                NewtypeContainer::Vec,
                NewtypeContainer::MapKey,
            ]])),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        cast_large_ints_to_i64: true,
        cast_f32_to_f64: true,
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    assert!(to_core.contains("fixture_core::SecretString::from(key)"), "{to_core}");
    assert!(
        to_core.contains("(value).into()"),
        "the JsSegment map value must convert to Segment:\n{to_core}"
    );

    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);
    assert!(from_core.contains("(key).into_inner()"), "{from_core}");
    assert!(
        from_core.contains("(value).into()"),
        "the Segment map value must convert to JsSegment:\n{from_core}"
    );
}

#[test]
fn excluded_flattened_optional_wrapper_requires_no_generated_conversion() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "HiddenReport".into(),
        rust_path: "fixture_core::HiddenReport".into(),
        fields: vec![FieldDef {
            name: "hidden_credential".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            binding_excluded: true,
            newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Optional,
            ]])),
            ..Default::default()
        }],
        has_default: true,
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);

    assert!(!to_core.contains("hidden_credential"), "{to_core}");
    assert!(!from_core.contains("hidden_credential"), "{from_core}");
}

#[test]
fn nested_optional_boxed_enum_fallback_defaults_the_complete_container() {
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeRef};

    let field = FieldDef {
        name: "_0".into(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        is_boxed: true,
        newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
        ]])),
        ..Default::default()
    };

    assert_eq!(
        super::transparent_newtypes::default_to_core(&field),
        "Default::default()"
    );
}
