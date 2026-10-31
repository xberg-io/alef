use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{build_variant_shapes, build_variant_shapes_with_names};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use crate::e2e::field_access::types::VariantShape;

fn definition(untagged: bool) -> EnumDef {
    EnumDef {
        name: "Content".to_string(),
        serde_untagged: untagged,
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                serde_rename: Some("wire-text".to_string()),
                fields: vec![FieldDef {
                    name: "body_text".to_string(),
                    ty: TypeRef::String,
                    serde_rename: Some("wireBody".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "FromText".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("Payload".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Class".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn shape_value(shape: &VariantShape) -> Value {
    let kinds: BTreeMap<_, _> = shape
        .wire_kinds
        .iter()
        .map(|(key, value)| (key, format!("{value:?}")))
        .collect();
    json!({
        "name": shape.name, "tuple": shape.is_tuple, "fields": shape.fields,
        "json_getter": shape.python_json_getter, "getter": shape.python_getter,
        "prefix": shape.python_wire_prefix, "wire_fields": shape.python_wire_fields,
        "wire_kinds": kinds, "untagged_newtype": shape.untagged_newtype,
    })
}

fn expected_shapes(untagged: bool) -> Value {
    let prefix = |name: &str| if untagged { vec![] } else { vec![name.to_string()] };
    json!([
        {
            "name": "Text", "tuple": false, "fields": [["body_text", null]],
            "json_getter": true, "getter": "text_payload", "prefix": prefix("wire-text"),
            "wire_fields": {"body_text": "wireBody"}, "wire_kinds": {"body_text": "String"},
            "untagged_newtype": false,
        },
        {
            "name": "FromText", "tuple": true, "fields": [["_0", "Payload"]],
            "json_getter": false, "getter": "from_text", "prefix": prefix("FromText"),
            "wire_fields": {"_0": "_0"}, "wire_kinds": {"_0": "Unknown"},
            "untagged_newtype": untagged,
        },
        {
            "name": "Class", "tuple": false, "fields": [], "json_getter": true,
            "getter": "class_", "prefix": prefix("Class"), "wire_fields": {},
            "wire_kinds": {}, "untagged_newtype": false,
        },
    ])
}

#[test]
fn preserves_every_shape_field_for_external_and_untagged_enums() {
    for untagged in [false, true] {
        let shapes = build_variant_shapes(&[definition(untagged)]);
        let values: Vec<_> = shapes["Content"].iter().map(shape_value).collect();
        assert_eq!(values.len(), 3);
        assert_eq!(json!(values), expected_shapes(untagged));
    }
}

#[test]
fn constructs_one_payload_name_table_per_enum_instead_of_per_variant() {
    let enums: Vec<_> = ["Content", "Other"]
        .into_iter()
        .map(|name| {
            let mut item = definition(false);
            item.name = name.to_string();
            item.variants = (0..64)
                .map(|index| EnumVariant {
                    name: format!("Value{index}"),
                    serde_rename: None,
                    ..item.variants[0].clone()
                })
                .collect();
            item
        })
        .collect();
    let mut calls = Vec::new();
    let shapes = build_variant_shapes_with_names(&enums, |definition| {
        calls.push(definition.name.clone());
        crate::codegen::generators::enums::dict_getter_py_names(definition)
    });
    assert_eq!(calls, ["Content", "Other"]);
    assert_eq!(shapes.len(), 2);
    assert_eq!(shapes["Content"].len(), 64);
    assert_eq!(shapes["Other"].len(), 64);
}

#[test]
#[should_panic(expected = "payload getter count must match enum variants")]
fn refuses_a_partial_name_table_instead_of_dropping_variant_shapes() {
    build_variant_shapes_with_names(&[definition(false)], |_| Vec::new());
}
