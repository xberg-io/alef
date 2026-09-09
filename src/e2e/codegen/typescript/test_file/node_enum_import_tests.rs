use super::*;

fn render_format_enum() -> EnumDef {
    EnumDef {
        name: "RenderFormat".into(),
        serde_tag: Some("content-type".into()),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![
            crate::core::ir::EnumVariant {
                name: "PlainText".into(),
                serde_rename: Some("text/plain".into()),
                ..Default::default()
            },
            crate::core::ir::EnumVariant {
                name: "Custom".into(),
                is_tuple: true,
                fields: vec![crate::core::ir::FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn not_error_assertion() -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }
}

#[test]
fn node_imports_enum_values_referenced_by_typed_inputs() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "processDocument".to_string();
    e2e_config.call.args = vec![crate::e2e::config::ArgMapping {
        name: "input".to_string(),
        field: "input".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("DocumentInput".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "process_document".to_string(),
        category: Some("document".to_string()),
        description: "process a document".to_string(),
        input: serde_json::json!({"kind": "uri"}),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let input_type = TypeDef {
        name: "DocumentInput".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("InputKind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let enums = [EnumDef {
        name: "InputKind".into(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Uri".into(),
            serde_rename: Some("uri".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let output = render_test_file(
        "node",
        "document",
        &[&fixture],
        "",
        "sample-bindings",
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[input_type],
        &enums,
        &[],
        "",
        &Default::default(),
        &[],
    );

    let binding_import = output
        .lines()
        .find(|line| line.contains("from \"sample-bindings\""))
        .expect("generated test must import its bindings");
    assert!(
        binding_import
            .split([',', '{', '}', ' '])
            .any(|token| token == "InputKind"),
        "Node must import enum classes used as runtime values: {binding_import}"
    );
    assert!(output.contains("kind: InputKind.Uri"), "{output}");
}

#[test]
fn node_imports_tagged_enum_type_used_by_object_literal_cast() {
    let argument = ArgMapping {
        name: "options".into(),
        field: "input.options".into(),
        arg_type: "json_object".into(),
        element_type: Some("RenderOptions".into()),
        optional: false,
        owned: false,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "render".into();
    e2e_config.call.args = vec![argument.clone()];
    let fixture = Fixture {
        id: "render_plain_text".into(),
        description: "render plain text".into(),
        input: serde_json::json!({"options": {"format": "text/plain"}}),
        assertions: vec![not_error_assertion()],
        ..Default::default()
    };
    let options = TypeDef {
        name: "RenderOptions".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "format".into(),
            ty: TypeRef::Named("RenderFormat".into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let output = render_test_file(
        "node",
        "render",
        &[&fixture],
        "",
        "sample-bindings",
        "render",
        &[argument],
        None,
        None,
        &e2e_config,
        &[options],
        &[render_format_enum()],
        &[],
        "",
        &Default::default(),
        &[],
    );

    let binding_import = output
        .lines()
        .find(|line| line.contains("from \"sample-bindings\""))
        .expect("generated test must import its bindings");
    assert!(
        binding_import.contains("type RenderFormat"),
        "tagged enum union must use a type-only import: {binding_import}"
    );
    assert!(
        output.contains("{ \"content-type\": \"text/plain\" } as RenderFormat"),
        "{output}"
    );
    assert!(!output.contains("RenderFormat.PlainText"), "{output}");
}

#[test]
fn node_lowers_direct_array_of_tagged_unit_variants() {
    let argument = ArgMapping {
        name: "formats".into(),
        field: "input.formats".into(),
        arg_type: "json_object".into(),
        element_type: Some("RenderFormat".into()),
        optional: false,
        owned: false,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "renderMany".into();
    e2e_config.call.args = vec![argument.clone()];
    let fixture = Fixture {
        id: "render_many".into(),
        description: "render many formats".into(),
        input: serde_json::json!({"formats": ["text/plain"]}),
        assertions: vec![not_error_assertion()],
        ..Default::default()
    };

    let output = render_test_file(
        "node",
        "render",
        &[&fixture],
        "",
        "sample-bindings",
        "renderMany",
        &[argument],
        None,
        None,
        &e2e_config,
        &[],
        &[render_format_enum()],
        &[],
        "",
        &Default::default(),
        &[],
    );

    assert!(
        output.contains("[{ \"content-type\": \"text/plain\" } as RenderFormat]"),
        "{output}"
    );
    assert!(!output.contains("[\"plain_text\"]"), "{output}");
    let binding_import = output
        .lines()
        .find(|line| line.contains("from \"sample-bindings\""))
        .expect("generated test must import its bindings");
    assert!(binding_import.contains("type RenderFormat"), "{binding_import}");
}

#[test]
fn node_preserves_direct_array_of_untagged_string_payloads() {
    let argument = ArgMapping {
        name: "inputs".into(),
        field: "input.inputs".into(),
        arg_type: "json_object".into(),
        element_type: Some("EmbeddingInput".into()),
        optional: false,
        owned: false,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "embedMany".into();
    e2e_config.call.args = vec![argument.clone()];
    let fixture = Fixture {
        id: "embed_many".into(),
        description: "embed many inputs".into(),
        input: serde_json::json!({"inputs": ["hello"]}),
        assertions: vec![not_error_assertion()],
        ..Default::default()
    };
    let untagged = EnumDef {
        name: "EmbeddingInput".into(),
        serde_untagged: true,
        variants: vec![crate::core::ir::EnumVariant {
            name: "Text".into(),
            is_tuple: true,
            fields: vec![crate::core::ir::FieldDef {
                name: "_0".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let output = render_test_file(
        "node",
        "embedding",
        &[&fixture],
        "",
        "sample-bindings",
        "embedMany",
        &[argument],
        None,
        None,
        &e2e_config,
        &[],
        &[untagged],
        &[],
        "",
        &Default::default(),
        &[],
    );

    assert!(output.contains("embedMany([\"hello\"])"), "{output}");
    assert!(!output.contains("EmbeddingInput.Hello"), "{output}");
}

#[test]
fn node_lowers_external_tagged_unit_and_payload_array_elements() {
    let argument = ArgMapping {
        name: "choices".into(),
        field: "input.choices".into(),
        arg_type: "json_object".into(),
        element_type: Some("ExternalChoice".into()),
        optional: false,
        owned: false,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chooseMany".into();
    e2e_config.call.args = vec![argument.clone()];
    let fixture = Fixture {
        id: "choose_many".into(),
        description: "choose unit and payload variants".into(),
        input: serde_json::json!({
            "choices": ["plain", {"wrapped": {"value": "payload"}}]
        }),
        assertions: vec![not_error_assertion()],
        ..Default::default()
    };
    let payload = TypeDef {
        name: "ChoicePayload".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "value".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let choice = EnumDef {
        name: "ExternalChoice".into(),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![
            crate::core::ir::EnumVariant {
                name: "Plain".into(),
                ..Default::default()
            },
            crate::core::ir::EnumVariant {
                name: "Wrapped".into(),
                is_tuple: true,
                fields: vec![crate::core::ir::FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("ChoicePayload".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let output = render_test_file(
        "node",
        "choice",
        &[&fixture],
        "",
        "sample-bindings",
        "chooseMany",
        &[argument],
        None,
        None,
        &e2e_config,
        &[payload],
        &[choice],
        &[],
        "",
        &Default::default(),
        &[],
    );

    assert!(
        output.contains(
            "[{ type: \"plain\" } as ExternalChoice, { type: \"wrapped\", wrapped: { value: \"payload\" } } as ExternalChoice]"
        ),
        "{output}"
    );
}

#[test]
fn node_imports_untagged_object_array_constraints_as_types() {
    let argument = ArgMapping {
        name: "choices".into(),
        field: "input.choices".into(),
        arg_type: "json_object".into(),
        element_type: Some("ObjectChoice".into()),
        optional: false,
        owned: false,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chooseMany".into();
    e2e_config.call.args = vec![argument.clone()];
    let fixture = Fixture {
        id: "choose_objects".into(),
        description: "choose object payloads".into(),
        input: serde_json::json!({"choices": [{"value": "payload"}]}),
        assertions: vec![not_error_assertion()],
        ..Default::default()
    };
    let choice = EnumDef {
        name: "ObjectChoice".into(),
        serde_untagged: true,
        variants: vec![crate::core::ir::EnumVariant {
            name: "Value".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let output = render_test_file(
        "node",
        "choice",
        &[&fixture],
        "",
        "sample-bindings",
        "chooseMany",
        &[argument],
        None,
        None,
        &e2e_config,
        &[],
        &[choice],
        &[],
        "",
        &Default::default(),
        &[],
    );

    assert!(output.contains("type ObjectChoice"), "{output}");
    assert!(
        output.contains("{ value: \"payload\" } satisfies ObjectChoice"),
        "{output}"
    );
}
