use super::*;

const NESTED: &str =
    "val.ink_list.map(|v| v.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect())";

fn nested_named() -> TypeRef {
    TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
        "AnnotationPoint".into(),
    )))))
}

#[test]
fn nested_vec_of_named_converts_each_inner_element_in_both_directions() {
    let ty = nested_named();
    let wrapped = TypeRef::Optional(Box::new(ty.clone()));
    for (ty, optional) in [(&ty, true), (&wrapped, false)] {
        let from = field_conversion_from_core("ink_list", ty, optional, false, &AHashSet::new());
        let to = field_conversion_to_core("ink_list", ty, optional);
        assert!(from.contains(NESTED), "{from}");
        assert!(to.contains(NESTED), "{to}");
    }
    let from = field_conversion_from_core("ink_list", &ty, false, false, &AHashSet::new());
    let to = field_conversion_to_core("ink_list", &ty, false);
    let required =
        "ink_list: val.ink_list.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect()";
    assert_eq!(from, required);
    assert_eq!(to, required);
}

#[test]
fn nested_vec_of_named_converts_through_the_cfg_entry_points() {
    let ty = nested_named();
    let config = ConversionConfig::default();
    let from = field_conversion_from_core_cfg("ink_list", &ty, true, false, &AHashSet::new(), &config);
    let to = field_conversion_to_core_cfg("ink_list", &ty, true, &config);
    assert!(from.contains(NESTED), "{from}");
    assert!(to.contains(NESTED), "{to}");
}

#[test]
fn nested_vec_of_primitives_keeps_the_flat_collect() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
    let from = field_conversion_from_core("rows", &ty, false, false, &AHashSet::new());
    assert_eq!(from, "rows: val.rows.into_iter().collect()");
}
