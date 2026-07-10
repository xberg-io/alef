use super::gen_wire_schema_consts;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;

fn named_field(name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        ..Default::default()
    }
}

fn string_field(name: &str, serde_rename: Option<&str>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        serde_rename: serde_rename.map(str::to_string),
        ..Default::default()
    }
}

fn config_type(name: &str, rename_all: Option<&str>, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("crate::{name}"),
        fields,
        has_default: true,
        has_serde: true,
        serde_rename_all: rename_all.map(str::to_string),
        ..Default::default()
    }
}

/// A data enum with one struct variant carrying the given config DTO as its payload — the seed for
/// the reachability walk.
fn enum_with_payload(field_type: &str) -> EnumDef {
    EnumDef {
        name: "ModelType".to_string(),
        rust_path: "crate::ModelType".to_string(),
        has_serde: true,
        variants: vec![EnumVariant {
            name: "Run".to_string(),
            fields: vec![named_field("config", field_type)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn surface(types: Vec<TypeDef>, enums: Vec<EnumDef>) -> ApiSurface {
    ApiSurface {
        types,
        enums,
        ..Default::default()
    }
}

#[test]
fn per_field_rename_is_carried_to_wire_name() {
    let cfg = config_type(
        "RunConfig",
        None,
        vec![string_field("max_chars", Some("maxCharacters"))],
    );
    let api = surface(vec![cfg], vec![enum_with_payload("RunConfig")]);
    let coercible: AHashSet<&str> = ["RunConfig"].into_iter().collect();

    let generated = gen_wire_schema_consts(&api, &coercible);

    assert!(
        generated.contains("const __ALEF_WIRE_RUN_CONFIG: &[__AlefAlias] = &["),
        "{generated}"
    );
    assert!(
        generated.contains(
            r#"__AlefAlias { rust: "max_chars", wire: "maxCharacters", kind: __AlefKind::Leaf, nested: &[] }"#
        ),
        "{generated}"
    );
}

#[test]
fn struct_level_rename_all_is_applied_to_wire_names() {
    let cfg = config_type(
        "RunConfig",
        Some("camelCase"),
        vec![string_field("model_id", None), string_field("region", None)],
    );
    let api = surface(vec![cfg], vec![enum_with_payload("RunConfig")]);
    let coercible: AHashSet<&str> = ["RunConfig"].into_iter().collect();

    let generated = gen_wire_schema_consts(&api, &coercible);

    assert!(
        generated.contains(r#"__AlefAlias { rust: "model_id", wire: "modelId", kind: __AlefKind::Leaf, nested: &[] }"#),
        "{generated}"
    );
    assert!(!generated.contains(r#"rust: "region""#), "{generated}");
}

#[test]
fn nested_dto_recurses_with_its_own_schema_const() {
    let inner = config_type("InnerConfig", None, vec![string_field("api_key", Some("apiKey"))]);
    let outer = config_type("OuterConfig", None, vec![named_field("inner", "InnerConfig")]);
    let api = surface(vec![outer, inner], vec![enum_with_payload("OuterConfig")]);
    let coercible: AHashSet<&str> = ["OuterConfig", "InnerConfig"].into_iter().collect();

    let generated = gen_wire_schema_consts(&api, &coercible);

    assert!(
        generated.contains(
            r#"__AlefAlias { rust: "inner", wire: "inner", kind: __AlefKind::Object, nested: __ALEF_WIRE_INNER_CONFIG }"#
        ),
        "{generated}"
    );
    assert!(
        generated.contains("const __ALEF_WIRE_INNER_CONFIG: &[__AlefAlias] = &["),
        "{generated}"
    );
    assert!(
        generated.contains(r#"__AlefAlias { rust: "api_key", wire: "apiKey", kind: __AlefKind::Leaf, nested: &[] }"#),
        "{generated}"
    );
}

#[test]
fn vec_and_optional_of_dto_use_seq_kind() {
    let item = config_type("ItemConfig", None, vec![string_field("item_id", Some("itemId"))]);
    let outer = config_type(
        "OuterConfig",
        None,
        vec![FieldDef {
            name: "items".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
                "ItemConfig".to_string(),
            ))))),
            ..Default::default()
        }],
    );
    let api = surface(vec![outer, item], vec![enum_with_payload("OuterConfig")]);
    let coercible: AHashSet<&str> = ["OuterConfig", "ItemConfig"].into_iter().collect();

    let generated = gen_wire_schema_consts(&api, &coercible);

    assert!(
        generated.contains(
            r#"__AlefAlias { rust: "items", wire: "items", kind: __AlefKind::Seq, nested: __ALEF_WIRE_ITEM_CONFIG }"#
        ),
        "{generated}"
    );
}

#[test]
fn cyclic_type_graph_breaks_back_edge() {
    let node = config_type(
        "NodeConfig",
        None,
        vec![
            string_field("node_name", Some("nodeName")),
            FieldDef {
                name: "child".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("NodeConfig".to_string()))),
                ..Default::default()
            },
        ],
    );
    let api = surface(vec![node], vec![enum_with_payload("NodeConfig")]);
    let coercible: AHashSet<&str> = ["NodeConfig"].into_iter().collect();

    let generated = gen_wire_schema_consts(&api, &coercible);

    assert_eq!(
        generated.matches("const __ALEF_WIRE_NODE_CONFIG:").count(),
        1,
        "{generated}"
    );
    assert!(
        generated.contains(r#"__AlefAlias { rust: "child", wire: "child", kind: __AlefKind::Object, nested: &[] }"#),
        "{generated}"
    );
    assert!(
        generated
            .contains(r#"__AlefAlias { rust: "node_name", wire: "nodeName", kind: __AlefKind::Leaf, nested: &[] }"#),
        "{generated}"
    );
}

#[test]
fn no_coercible_types_emits_nothing() {
    let api = surface(vec![], vec![]);
    let coercible: AHashSet<&str> = AHashSet::new();
    assert!(gen_wire_schema_consts(&api, &coercible).is_empty());
}
