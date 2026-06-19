use super::*;
use crate::core::ir::TypeRef;

#[test]
fn test_swift_type_name_bool_returns_bool() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::Bool)), "Bool");
}

#[test]
fn test_swift_type_name_usize_returns_uint() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::Usize)), "UInt");
}

#[test]
fn test_swift_type_name_u8_returns_uint8() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U8)), "UInt8");
}

#[test]
fn test_swift_type_name_u32_returns_uint32() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U32)), "UInt32");
}

#[test]
fn test_swift_type_name_u64_returns_uint64() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U64)), "UInt64");
}

#[test]
fn test_swift_type_name_i32_returns_int32() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::I32)), "Int32");
}

#[test]
fn test_swift_type_name_f32_returns_float() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::F32)), "Float");
}

fn make_function(name: &str, params: Vec<(&str, TypeRef)>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        params: params
            .into_iter()
            .map(|(pname, ty)| crate::core::ir::ParamDef {
                name: pname.to_string(),
                ty,
                ..crate::core::ir::ParamDef::default()
            })
            .collect(),
        return_type,
        ..FunctionDef::default()
    }
}

#[test]
fn skips_forwarder_when_param_type_is_excluded() {
    let func = make_function(
        "extract_keywords",
        vec![("config", TypeRef::Named("KeywordConfig".to_string()))],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("KeywordConfig".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_return_type_is_excluded() {
    let func = make_function(
        "build_keyword",
        vec![("text", TypeRef::String)],
        TypeRef::Named("Keyword".to_string()),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn keeps_forwarder_when_only_primitives_are_used() {
    let func = make_function(
        "echo_count",
        vec![("count", TypeRef::Primitive(PrimitiveType::U32))],
        TypeRef::Primitive(PrimitiveType::U32),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("KeywordConfig".to_string());
    exclude.insert("Keyword".to_string());
    assert!(!function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_vec_named_param_is_excluded() {
    let func = make_function(
        "score_keywords",
        vec![(
            "keywords",
            TypeRef::Vec(Box::new(TypeRef::Named("Keyword".to_string()))),
        )],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_optional_named_return_is_excluded() {
    let func = make_function(
        "maybe_yake",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("YakeParams".to_string()))),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("YakeParams".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn empty_exclude_set_keeps_every_function() {
    let func = make_function(
        "extract_keywords",
        vec![("config", TypeRef::Named("KeywordConfig".to_string()))],
        TypeRef::Named("Keyword".to_string()),
    );
    let exclude: HashSet<String> = HashSet::new();
    assert!(!function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_map_value_is_excluded_type() {
    let func = make_function(
        "score_map",
        vec![(
            "table",
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Keyword".to_string())),
            ),
        )],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}
