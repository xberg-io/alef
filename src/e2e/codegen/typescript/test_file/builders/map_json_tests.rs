use super::*;
use crate::core::ir::FieldDef;

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        ..Default::default()
    }
}

fn types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "FixtureRequest".into(),
            fields: vec![
                field(
                    "metadata",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                ),
                field("extra_body", TypeRef::Json),
                field(
                    "items",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Item".into()))),
                ),
                field("request_id", TypeRef::String),
            ],
            ..Default::default()
        },
        TypeDef {
            name: "Item".into(),
            fields: vec![field("display_name", TypeRef::String)],
            ..Default::default()
        },
    ]
}

fn value() -> serde_json::Value {
    serde_json::json!({
        "metadata": {"trace_id": "first", "traceId": "second", "__proto__": "own value"},
        "extra_body": {"properties": {"postal_code": {"type": "string"}}, "__proto__": {"polluted": true}},
        "items": {"first_item": {"display_name": "First"}},
        "request_id": "request"
    })
}

fn node_expression() -> String {
    node_typed_value_expression(
        &value(),
        "FixtureRequest",
        &Default::default(),
        &types(),
        &[],
        &mut Default::default(),
    )
}

fn assert_keys(expression: &str) {
    assert!(expression.contains("trace_id:"), "{expression}");
    assert!(expression.contains("traceId:"), "{expression}");
    assert!(expression.contains("postal_code:"), "{expression}");
    assert!(expression.contains("first_item:"), "{expression}");
    assert!(expression.contains("displayName:"), "{expression}");
    assert!(expression.contains("requestId:"), "{expression}");
    assert!(expression.contains(r#"["__proto__"]:"#), "{expression}");
}

#[test]
fn node_typed_inputs_preserve_data_keys_and_rename_declared_fields() {
    assert_keys(&node_expression());
}

#[test]
fn generic_typed_inputs_preserve_map_values_and_json_keys() {
    assert_keys(&json_to_js_camel_with_types(&value(), Some("FixtureRequest"), &types()));
}

#[test]
fn json_literals_preserve_proto_as_an_own_data_property() {
    let value = serde_json::json!({"__proto__": {"marker": "value"}});
    assert!(json_to_js(&value).contains(r#"["__proto__"]:"#));
    assert!(json_to_js_multiline(&value, 0).contains(r#"["__proto__"]:"#));
}

#[test]
#[ignore = "requires tsc and Node; executed explicitly in release verification"]
fn generated_node_map_inputs_typecheck_and_preserve_exact_runtime_data() {
    let root = tempfile::tempdir().expect("fixture directory");
    let source = format!(
        r#"interface Item {{ displayName: string }}
interface FixtureRequest {{ metadata: Record<string, string>; extraBody: Record<string, unknown>; items: Record<string, Item>; requestId: string }}
const value: FixtureRequest = {};
if (!Object.hasOwn(value.metadata, "__proto__") || !Object.hasOwn(value.extraBody, "__proto__")) throw new Error("missing own key");
if (Object.getPrototypeOf(value.extraBody) !== Object.prototype) throw new Error("prototype changed");
console.log(JSON.stringify(value));"#,
        node_expression()
    );
    let file = root.path().join("input.ts");
    std::fs::write(&file, source).expect("generated TypeScript");
    let compiled = std::process::Command::new("tsc")
        .args(["--strict", "--target", "ES2022", "--module", "commonjs"])
        .arg(&file)
        .output()
        .expect("run real tsc");
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = std::process::Command::new("node")
        .arg(root.path().join("input.js"))
        .output()
        .expect("run generated JavaScript");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON result");
    let mut expected = value();
    let object = expected.as_object_mut().expect("object");
    let extra = object.remove("extra_body").expect("extra_body");
    object.insert("extraBody".into(), extra);
    let id = object.remove("request_id").expect("request_id");
    object.insert("requestId".into(), id);
    object.insert(
        "items".into(),
        serde_json::json!({"first_item": {"displayName": "First"}}),
    );
    assert_eq!(actual, expected);
}

#[test]
fn unknown_struct_types_retain_legacy_camel_field_lowering() {
    let value = serde_json::json!({"display_name": "First"});
    for type_name in [None, Some("Unregistered")] {
        assert_eq!(
            json_to_js_camel_with_types(&value, type_name, &types()),
            "{ displayName: \"First\" }"
        );
    }
}

#[test]
fn root_arrays_retain_the_declared_item_type() {
    let value = serde_json::json!([{"display_name": "First"}]);
    assert_eq!(
        json_to_js_camel_with_types(&value, Some("Item"), &types()),
        "[{ displayName: \"First\" }]"
    );
}

#[test]
fn map_entry_names_do_not_trigger_struct_field_enum_overrides() {
    let enum_fields = [("mode".into(), "Mode".into())].into_iter().collect();
    let value = serde_json::json!({"metadata": {"mode": "safe"}});
    let expression = node_typed_value_expression(
        &value,
        "FixtureRequest",
        &enum_fields,
        &types(),
        &[],
        &mut Default::default(),
    );
    assert_eq!(expression, "{ metadata: { mode: \"safe\" } }");
}

#[test]
fn map_container_values_retain_struct_types_and_optional_json_identity() {
    let definitions = vec![
        TypeDef {
            name: "Containers".into(),
            fields: vec![
                field(
                    "items",
                    TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Named(
                            "Item".into(),
                        )))))),
                    ),
                ),
                field("payload", TypeRef::Optional(Box::new(TypeRef::Json))),
            ],
            ..Default::default()
        },
        types().remove(1),
    ];
    let value =
        serde_json::json!({"items": {"first_item": [null, {"display_name": "First"}]}, "payload": {"raw_key": true}});
    let expected = "{ items: { first_item: [null, { displayName: \"First\" }] }, payload: { raw_key: true } }";
    assert_eq!(
        json_to_js_camel_with_types(&value, Some("Containers"), &definitions),
        expected
    );
    assert_eq!(
        node_typed_value_expression(
            &value,
            "Containers",
            &Default::default(),
            &definitions,
            &[],
            &mut Default::default()
        ),
        expected
    );
}

#[test]
fn typed_map_enum_values_keep_declared_members_and_imports() {
    let definitions = [TypeDef {
        name: "EnumValues".into(),
        fields: vec![field(
            "values",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Mode".into()))),
        )],
        ..Default::default()
    }];
    let enums = [EnumDef {
        name: "Mode".into(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Safe".into(),
            serde_rename: Some("safe".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut imports = Default::default();
    let expression = node_typed_value_expression(
        &serde_json::json!({"values": {"entry_key": "safe"}}),
        "EnumValues",
        &Default::default(),
        &definitions,
        &enums,
        &mut imports,
    );
    assert_eq!(expression, "{ values: { entry_key: Mode.Safe } }");
    assert_eq!(imports, ["Mode".to_string()].into_iter().collect());
}

#[test]
fn unknown_fields_on_known_structs_retain_legacy_camel_lowering() {
    let value = serde_json::json!({"unknown_field": {"nested_value": true}});
    assert_eq!(
        json_to_js_camel_with_types(&value, Some("FixtureRequest"), &types()),
        "{ unknownField: { nestedValue: true } }"
    );
}
