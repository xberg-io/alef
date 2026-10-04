use super::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::ir::{
    EnumDef, EnumVariant, FieldDef, NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, TypeRef,
};

fn transparent_string_wrapper(paths: Vec<Vec<NewtypeContainer>>) -> String {
    NewtypeWrapper::encode_explicit(
        &paths
            .into_iter()
            .map(|containers| {
                NewtypeWrapperMetadata::transparent_string("test_core::SecretString", "from", "into_inner", containers)
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn tagged_enum_transparent_string_fields_convert_without_wrapper_default() {
    let field = |name: &str, ty: TypeRef, optional: bool, is_boxed: bool, paths| FieldDef {
        name: name.to_owned(),
        ty,
        optional,
        is_boxed,
        newtype_wrapper: Some(transparent_string_wrapper(paths)),
        ..FieldDef::default()
    };
    let en = EnumDef {
        name: "Authentication".to_owned(),
        rust_path: "test_core::Authentication".to_owned(),
        serde_tag: Some("type".to_owned()),
        variants: vec![
            EnumVariant {
                name: "Bearer".to_owned(),
                is_tuple: true,
                fields: vec![field("_0", TypeRef::String, false, false, vec![vec![]])],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Optional".to_owned(),
                fields: vec![field(
                    "value",
                    TypeRef::String,
                    true,
                    false,
                    vec![vec![NewtypeContainer::Optional]],
                )],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Values".to_owned(),
                fields: vec![field(
                    "values",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                    false,
                    vec![vec![NewtypeContainer::MapKey], vec![NewtypeContainer::MapValue]],
                )],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Boxed".to_owned(),
                fields: vec![field(
                    "boxed",
                    TypeRef::String,
                    true,
                    true,
                    vec![vec![NewtypeContainer::Optional]],
                )],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };
    let struct_names = ahash::AHashSet::new();

    let binding_to_core = gen_tagged_enum_binding_to_core(&en, "test_core", "Js", &struct_names, &[]);
    assert!(
        binding_to_core.contains("test_core::SecretString::from"),
        "binding payloads must construct the wrapper:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("(val.value).map(test_core::SecretString::from)"),
        "root optional enum payload must use the constructor function directly:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("map(|value| test_core::SecretString::from(value))"),
        "root optional enum payload must not emit Clippy's redundant-closure shape:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("_ => Self::Bearer(test_core::SecretString::from(Default::default()))"),
        "the unknown-tag fallback must construct a wrapped default without requiring the wrapper itself to implement Default:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("_ => Self::Bearer(Default::default())"),
        "the transparent wrapper deliberately has no Default bound:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains(".map(Box::new)"),
        "an optional boxed wrapper must box the mapped payload:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("Box::new((val.boxed).map"),
        "the Option itself must not be boxed:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&en, "test_core", "Js", &struct_names, None, &[]);
    assert_eq!(
        core_to_binding.matches("into_inner()").count(),
        5,
        "each transparent wrapper payload must be consumed exactly once:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("boxed.map(|value| *value)"),
        "an optional boxed wrapper must be dereferenced inside Option::map:\n{core_to_binding}"
    );
    for payload in ["bearer", "value", "key", "boxed"] {
        assert!(
            !core_to_binding.contains(&format!("{payload}.to_string()")),
            "wrapper payload `{payload}` must not require Display:\n{core_to_binding}"
        );
    }
}
