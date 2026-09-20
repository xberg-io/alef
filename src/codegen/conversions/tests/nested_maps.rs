use super::*;

#[test]
fn named_map_keys_and_values_convert_in_both_directions() {
    let ty = TypeRef::Map(
        Box::new(TypeRef::Named("SourceRole".into())),
        Box::new(TypeRef::Named("OperationSource".into())),
    );
    for optional in [false, true] {
        let from = field_conversion_from_core("sources", &ty, optional, false, &AHashSet::new());
        let to = field_conversion_to_core("sources", &ty, optional);
        assert!(from.contains("(k.into(), v.into())"), "{from}");
        assert!(to.contains("(k.into(), v.into())"), "{to}");
    }
    let wrapped = TypeRef::Optional(Box::new(ty.clone()));
    assert_eq!(
        field_conversion_to_core("sources", &wrapped, false),
        field_conversion_to_core("sources", &ty, true)
    );
    assert_eq!(
        field_conversion_from_core("sources", &wrapped, false, false, &AHashSet::new()),
        field_conversion_from_core("sources", &ty, true, false, &AHashSet::new())
    );
}

#[test]
fn nested_json_maps_convert_each_value_without_dropping_entries() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Json),
    )));
    for optional in [false, true] {
        let from = field_conversion_from_core("records", &ty, optional, false, &AHashSet::new());
        let to = field_conversion_to_core("records", &ty, optional);
        assert_eq!(
            field_conversion_from_core_cfg(
                "records",
                &ty,
                optional,
                false,
                &AHashSet::new(),
                &ConversionConfig::default()
            ),
            from
        );
        assert_eq!(
            field_conversion_to_core_cfg("records", &ty, optional, &ConversionConfig::default()),
            to
        );
        let prefix = if optional {
            "records: val.records.map(|items| items.into_iter()"
        } else {
            "records: val.records.into_iter()"
        };
        assert!(from.starts_with(prefix), "{from}");
        assert!(to.starts_with(prefix), "{to}");
        assert!(from.contains("(k, v.to_string())"), "{from}");
        assert!(
            to.contains("serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v))"),
            "{to}"
        );
        assert!(!from.contains("filter_map"));
        assert!(!to.contains("filter_map"));
    }
    let wrapped = TypeRef::Optional(Box::new(ty.clone()));
    assert_eq!(
        field_conversion_from_core("records", &wrapped, false, false, &AHashSet::new()),
        field_conversion_from_core("records", &ty, true, false, &AHashSet::new())
    );
    assert_eq!(
        field_conversion_to_core("records", &wrapped, false),
        field_conversion_to_core("records", &ty, true)
    );
}
