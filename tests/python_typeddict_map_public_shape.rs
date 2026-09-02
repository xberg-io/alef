use std::collections::{HashMap, HashSet};

use alef::core::ir::{FieldDef, TypeDef, TypeRef};
use alef::e2e::field_access::{FieldResolver, PythonTypedDictMap};

#[test]
fn pre_0793_public_struct_literal_remains_source_compatible() {
    let map = PythonTypedDictMap {
        typeddict_types: HashSet::new(),
        field_types: HashMap::new(),
        root_type: Some("Report".to_string()),
    };

    assert_eq!(map.root_type.as_deref(), Some("Report"));
    assert!(map.is_empty());
}

#[test]
fn generated_map_value_edges_are_not_observable_through_the_public_field_map() {
    let types = vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("Metadata".to_string())),
                ),
                ..FieldDef::default()
            }],
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metadata".to_string(),
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        },
    ];

    let map = FieldResolver::python_typeddict_fields(&types);

    assert_eq!(map.advance(Some("Report"), "\0map-value:entries"), None);
    assert!(
        map.field_types
            .values()
            .all(|fields| fields.keys().all(|key| !key.contains('\0')))
    );
}
