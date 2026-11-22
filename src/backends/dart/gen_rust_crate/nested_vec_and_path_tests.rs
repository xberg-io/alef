use super::mirror_conversions::emit_from_impl_for_struct;
use super::opaque::emit_opaque_impl_block;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

#[test]
fn nested_vec_of_named_converts_each_inner_element_into_the_mirror() {
    let ty = TypeDef {
        name: "Annotation".to_string(),
        rust_path: "mylib::Annotation".to_string(),
        fields: vec![FieldDef {
            name: "ink_list".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))))),
            optional: true,
            ..Default::default()
        }],
        is_clone: true,
        ..Default::default()
    };
    let mut out = String::new();
    emit_from_impl_for_struct(&mut out, &ty, "mylib");
    assert!(
        out.contains("inner.into_iter().map(Point::from).collect::<Vec<_>>()"),
        "got:\n{out}"
    );
    assert!(!out.contains("ink_list: v.ink_list,"), "got:\n{out}");
}

#[test]
fn static_opaque_method_calls_the_type_at_its_defining_module_path() {
    let param = ParamDef {
        name: "text".to_string(),
        ty: TypeRef::String,
        is_ref: true,
        ..Default::default()
    };
    let ty = TypeDef {
        name: "Parser".to_string(),
        rust_path: "mylib::inner::parser::Parser".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "parse_dimension".to_string(),
            is_static: true,
            params: vec![param],
            return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::F64))),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_paths: HashMap<String, String> =
        HashMap::from([("Parser".to_string(), "mylib::inner::parser::Parser".to_string())]);
    let mut out = String::new();
    emit_opaque_impl_block(
        &mut out,
        &ty,
        "mylib",
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &ResolvedCrateConfig::default(),
        &type_paths,
        &HashSet::new(),
        &[],
    )
    .unwrap();
    assert!(
        out.contains("mylib::inner::parser::Parser::parse_dimension("),
        "got:\n{out}"
    );
    assert!(!out.contains("mylib::Parser::"), "got:\n{out}");
}
