#[test]
fn legacy_vec_newtype_types_parenthesized_intermediate_collect() {
    let converted = super::render::type_intermediate_vec_collect("(val.children.into_iter().collect())");

    assert_eq!(
        converted, "(val.children.into_iter().collect::<Vec<_>>())",
        "the legacy newtype stage must type a parenthesized collection before iterating it again"
    );
}
