use super::builders::ts_builder_expression;
use super::snippet::{SnippetContext, render_snippet_body};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn arity_snippet(lang: &str, functions: &[FunctionDef]) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "list_models".into();
    e2e_config.call.module = "@example/library".into();
    let fixture = Fixture {
        id: "list_models".into(),
        input: serde_json::json!({"model": "local/any"}),
        ..Default::default()
    };
    render_snippet_body(SnippetContext {
        lang,
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &[],
        enums: &[],
        functions,
        wasm_type_prefix: "",
        config: &Default::default(),
    })
}

#[test]
fn known_zero_argument_calls_do_not_receive_fixture_setup_metadata() {
    let functions = [FunctionDef {
        name: "list_models".into(),
        ..Default::default()
    }];
    for lang in ["node", "wasm"] {
        let body = arity_snippet(lang, &functions);
        assert!(body.contains("listModels()"), "{lang}: {body}");
        assert!(!body.contains("listModels({"), "{lang}: {body}");
    }
}

#[test]
fn parameterized_calls_keep_their_fixture_argument() {
    let functions = [FunctionDef {
        name: "list_models".into(),
        params: vec![ParamDef {
            name: "query".into(),
            ty: TypeRef::Json,
            ..Default::default()
        }],
        ..Default::default()
    }];
    for lang in ["node", "wasm"] {
        let body = arity_snippet(lang, &functions);
        assert!(body.contains("listModels({ model: \"local/any\" })"), "{lang}: {body}");
    }
}

fn builder(lang: &str, input: serde_json::Value, fields: Vec<FieldDef>, enums: &[EnumDef]) -> String {
    let definitions = [TypeDef {
        name: "Request".into(),
        fields,
        ..Default::default()
    }];
    ts_builder_expression(
        input.as_object().expect("object"),
        if lang == "wasm" { "WasmRequest" } else { "Request" },
        &Default::default(),
        lang,
        &Default::default(),
        &Default::default(),
        &definitions,
        enums,
        "Wasm",
        &[],
        &mut Default::default(),
    )
}

#[test]
fn wasm_flattened_json_targets_declared_config_setter_and_preserves_wire_keys() {
    let fields = vec![
        FieldDef {
            name: "tool_type".into(),
            ty: TypeRef::String,
            serde_rename: Some("type".into()),
            ..Default::default()
        },
        FieldDef {
            name: "config".into(),
            ty: TypeRef::Json,
            serde_flatten: true,
            ..Default::default()
        },
    ];
    let expression = builder(
        "wasm",
        serde_json::json!({
            "type": "function", "name": "get_weather", "x-custom_key": {"inner_key": 42},
            "parameters": {"type": "object", "properties": {"city_name": {"type": "string"}}}
        }),
        fields,
        &[],
    );
    assert!(expression.contains("_u0.toolType = \"function\";"), "{expression}");
    assert!(expression.contains("_u0.config ="), "{expression}");
    assert!(!expression.contains("_u0.name ="), "{expression}");
    assert!(!expression.contains("_u0.parameters ="), "{expression}");
    assert!(expression.contains("inner_key"), "{expression}");
    assert!(expression.contains("city_name"), "{expression}");
}

#[test]
fn unknown_enum_wire_values_are_refused_instead_of_inventing_members() {
    let definition = EnumDef {
        name: "Purpose".into(),
        variants: vec![EnumVariant {
            name: "FineTune".into(),
            serde_rename: Some("fine-tune".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    for lang in ["node", "wasm"] {
        let _ = crate::e2e::codegen::fixture_refusal::take();
        let expression = builder(
            lang,
            serde_json::json!({"purpose": "invalid-purpose"}),
            vec![FieldDef {
                name: "purpose".into(),
                ty: TypeRef::Named("Purpose".into()),
                ..Default::default()
            }],
            std::slice::from_ref(&definition),
        );
        let error = crate::e2e::codegen::fixture_refusal::take_error(lang)
            .expect("undeclared enum value must refuse generation");
        assert!(error.to_string().contains("invalid-purpose"), "{error}");
        assert!(!expression.contains("InvalidPurpose"), "{expression}");
    }
}

#[test]
fn declared_renamed_enum_values_keep_the_actual_member() {
    let definition = EnumDef {
        name: "Purpose".into(),
        variants: vec![EnumVariant {
            name: "FineTune".into(),
            serde_rename: Some("fine-tune".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    for lang in ["node", "wasm"] {
        let _ = crate::e2e::codegen::fixture_refusal::take();
        let expression = builder(
            lang,
            serde_json::json!({"purpose": "fine-tune"}),
            vec![FieldDef {
                name: "purpose".into(),
                ty: TypeRef::Named("Purpose".into()),
                ..Default::default()
            }],
            std::slice::from_ref(&definition),
        );
        assert!(expression.contains("Purpose.FineTune"), "{expression}");
        assert!(crate::e2e::codegen::fixture_refusal::take_error(lang).is_none());
    }
}

#[test]
fn calls_without_ir_keep_the_historical_fixture_argument() {
    for lang in ["node", "wasm"] {
        let body = arity_snippet(lang, &[]);
        assert!(body.contains("listModels({ model: \"local/any\" })"), "{lang}: {body}");
    }
}

#[test]
fn ordinary_wasm_fields_keep_independent_setters() {
    let fields = vec![
        FieldDef {
            name: "name".into(),
            ty: TypeRef::String,
            ..Default::default()
        },
        FieldDef {
            name: "config".into(),
            ty: TypeRef::Json,
            ..Default::default()
        },
    ];
    let expression = builder(
        "wasm",
        serde_json::json!({"name": "weather", "config": {"city_name": "Berlin"}}),
        fields,
        &[],
    );
    assert!(expression.contains("_u0.name = \"weather\";"), "{expression}");
    assert!(expression.contains("_u0.config ="), "{expression}");
}

#[test]
fn flattened_map_runtime_preserves_special_and_nested_keys() {
    let expected = serde_json::json!({"__proto__": {"polluted": true}, "snake_case": {"inner_key": 42}});
    let fields = vec![FieldDef {
        name: "config".into(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
        serde_flatten: true,
        ..Default::default()
    }];
    let expression = builder("wasm", expected.clone(), fields, &[]);
    let source = format!(
        "class WasmRequest {{ static default() {{ return new WasmRequest(); }} }}; const result = {expression}; process.stdout.write(JSON.stringify(result.config));"
    );
    let output = crate::core::tool_command("node")
        .args(["-e", &source])
        .output()
        .expect("Node runtime required");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).expect("generated map JSON");
    assert_eq!(actual, expected);
}

fn request_message_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Request".into(),
            fields: vec![FieldDef {
                name: "messages".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Message".into()))),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "UserMessage".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn user_message_enum() -> EnumDef {
    EnumDef {
        name: "Message".into(),
        serde_tag: Some("role".into()),
        variants: vec![EnumVariant {
            name: "User".into(),
            serde_rename: Some("user".into()),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("UserMessage".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn node_doc_request_arrays_lower_tagged_tuple_payloads_through_the_declared_input_type() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chat".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.args = vec![
        serde_json::from_value(serde_json::json!({
            "name": "req", "field": "input", "type": "json_object", "owned": true
        }))
        .expect("argument config"),
    ];
    let types = request_message_types();
    let enums = [user_message_enum()];
    let functions = [FunctionDef {
        name: "chat".into(),
        params: vec![ParamDef {
            name: "req".into(),
            ty: TypeRef::Named("Request".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let fixture = Fixture {
        id: "chat".into(),
        input: serde_json::json!({"messages": [{"role": "user", "content": "proof"}]}),
        ..Default::default()
    };
    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &types,
        enums: &enums,
        functions: &functions,
        wasm_type_prefix: "",
        config: &Default::default(),
    });
    assert!(body.contains("user: { content: \"proof\" }"), "{body}");
    assert!(body.contains("role: \"user\""), "{body}");
}
