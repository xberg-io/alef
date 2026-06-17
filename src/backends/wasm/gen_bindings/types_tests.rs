use super::*;
use std::collections::HashMap;

fn mapper() -> WasmMapper {
    WasmMapper::new(HashMap::new(), "Wasm".to_string())
}

fn enum_names(names: &[&str]) -> AHashSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn gen_getter_option_vec_unit_enum_flattens_option() {
    let field = FieldDef {
        name: "modalities".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: true,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let out = gen_getter(&field, &mapper(), &enums, &tagged, false);
    assert!(
        out.contains("-> Option<Vec<String>>"),
        "getter must return Option<Vec<String>>: {out}"
    );
    assert!(
        out.contains("self.modalities.as_ref().map(|v| v.iter()"),
        "getter must flatten the Option via as_ref().map(...): {out}"
    );
    assert!(
        out.contains("x.to_api_str().to_owned()"),
        "getter must call to_api_str() on each element: {out}"
    );
}

#[test]
fn gen_setter_option_vec_unit_enum_wraps_some() {
    let field = FieldDef {
        name: "modalities".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: true,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let out = gen_setter(&field, &mapper(), &enums, false, &tagged);
    assert!(
        out.contains("value: Option<Vec<String>>"),
        "setter must take Option<Vec<String>>: {out}"
    );
    assert!(
        out.contains("value.map(|v| v.into_iter().filter_map(|s| WasmModality::from_api_str(&s)).collect())"),
        "setter must map the Option and collect into the inner Vec: {out}"
    );
}

#[test]
fn gen_getter_setter_required_vec_unit_enum_unchanged() {
    let field = FieldDef {
        name: "tags".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: false,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let getter = gen_getter(&field, &mapper(), &enums, &tagged, false);
    assert!(
        getter.contains("-> Vec<String>"),
        "getter must return Vec<String>: {getter}"
    );
    assert!(
        getter.contains("self.tags.iter().map(|v| v.to_api_str().to_owned()).collect()"),
        "getter must iterate the Vec directly: {getter}"
    );
    let setter = gen_setter(&field, &mapper(), &enums, false, &tagged);
    assert!(
        setter.contains("value: Vec<String>"),
        "setter must take Vec<String>: {setter}"
    );
    assert!(
        setter.contains("value.into_iter().filter_map(|s| WasmModality::from_api_str(&s)).collect()"),
        "setter must collect into the inner Vec: {setter}"
    );
    assert!(
        !setter.contains("value.map("),
        "required Vec setter must not Option::map: {setter}"
    );
}
