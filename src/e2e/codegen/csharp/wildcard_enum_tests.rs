//! Wildcard assertions must compare enum wire values rather than CLR member names.

use super::*;

fn wildcard_output(operator: &str, leaf_type: TypeRef, expected: &str) -> String {
    let (mut types, mut enums, functions) = table_ir();
    types[0].fields = vec![FieldDef {
        name: "links".into(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Link".into()))),
        ..FieldDef::default()
    }];
    types.push(TypeDef {
        name: "Link".into(),
        fields: vec![kind_field(leaf_type, false)],
        ..TypeDef::default()
    });
    enums[0].serde_rename_all = Some("snake_case".into());
    enums[0].variants[1].serde_rename = Some("Custom+Kind".into());
    let mut fixture = fixture_calling("process");
    fixture.assertions = vec![Assertion {
        assertion_type: operator.into(),
        field: Some("links[].kind".into()),
        value: (operator != "contains_all").then(|| serde_json::json!(expected)),
        values: (operator == "contains_all").then(|| vec![serde_json::json!(expected)]),
        ..Assertion::default()
    }];
    render(
        &fixture,
        &e2e_config_for("process", "process", |_| {}),
        &types,
        &enums,
        &functions,
    )
}

#[test]
fn wildcard_enum_assertions_use_the_registered_wire_converter() {
    for operator in ["contains", "not_contains", "contains_all"] {
        for expected in ["key_value", "Custom+Kind"] {
            let out = wildcard_output(operator, TypeRef::Named("DataNodeKind".into()), expected);
            assert!(out.contains("System.Text.Json.JsonSerializer.Serialize("), "{out}");
            assert!(out.contains("JavaScriptEncoder.UnsafeRelaxedJsonEscaping"), "{out}");
            assert!(!out.contains("Convert.ToString("), "{out}");
            assert!(out.contains(&format!(".Contains(\"{expected}\")")), "{out}");
        }
    }
}

#[test]
fn wildcard_enum_classification_keeps_string_fields_as_strings() {
    let out = wildcard_output("contains", TypeRef::String, "key_value");
    assert!(out.contains("Convert.ToString("), "{out}");
    assert!(!out.contains("System.Text.Json.JsonSerializer.Serialize("), "{out}");
}
