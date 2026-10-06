#[test]
fn legacy_vec_newtype_types_parenthesized_intermediate_collect() {
    let converted = crate::codegen::conversions::helpers::apply_field_newtype_to_core(
        "(val.children.into_iter().collect())",
        &crate::core::ir::TypeRef::Vec(Box::new(crate::core::ir::TypeRef::Primitive(
            crate::core::ir::PrimitiveType::Usize,
        ))),
        false,
        "sample_core::NodeIndex",
    );

    assert_eq!(
        converted, "((val.children.into_iter().collect::<Vec<_>>())).into_iter().map(sample_core::NodeIndex).collect()",
        "the legacy newtype conversion must type a parenthesized collection before iterating it again"
    );
}
