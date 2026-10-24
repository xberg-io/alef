use super::*;
use crate::core::ir::{EnumDef, FieldDef, TypeDef};

fn reply_api() -> ApiSurface {
    let message = TypeDef {
        name: "Message".into(),
        rust_path: "sample::Message".into(),
        has_serde: true,
        fields: vec![
            FieldDef {
                name: "content".into(),
                ty: TypeRef::Named("Content".into()),
                optional: true,
                ..Default::default()
            },
            FieldDef {
                name: "summary_text".into(),
                ty: TypeRef::Named("Content".into()),
                ..Default::default()
            },
            FieldDef {
                name: "label".into(),
                ty: TypeRef::String,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    ApiSurface {
        crate_name: "sample".into(),
        types: vec![message],
        enums: vec![EnumDef {
            name: "Content".into(),
            rust_path: "sample::Content".into(),
            has_serde: true,
            serde_untagged: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn emitted(api: &ApiSurface, text_types: &[&str], exclude_fields: &[&str]) -> String {
    try_emit(api, text_types, exclude_fields).expect("accessors are emitted")
}

fn try_emit(api: &ApiSurface, text_types: &[&str], exclude_fields: &[&str]) -> anyhow::Result<String> {
    let mut out = String::new();
    emit(
        api,
        &text_types.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
        &HashSet::new(),
        &exclude_fields.iter().map(|f| f.to_string()).collect(),
        &HashSet::new(),
        &mut out,
    )?;
    Ok(out)
}

#[test]
fn each_text_field_gets_an_accessor_that_decodes_its_json_getter() {
    let out = emitted(&reply_api(), &["Content"], &[]);
    assert!(out.starts_with("extension RustBridge.MessageRef {"), "{out}");
    assert!(out.contains("public func contentValue() throws -> Content? {"), "{out}");
    assert!(
        out.contains(
            "try contentJson().map { try JSONDecoder().decode(Content.self, from: Data($0.toString().utf8)) }"
        ),
        "{out}"
    );
    assert!(
        out.contains("public func summaryTextValue() throws -> Content {"),
        "{out}"
    );
    assert!(
        out.contains("try JSONDecoder().decode(Content.self, from: Data(summaryTextJson().toString().utf8))"),
        "{out}"
    );
    assert!(
        !out.contains("labelValue"),
        "a field outside the text types gets no accessor:\n{out}"
    );
}

#[test]
fn no_accessor_is_emitted_without_configured_text_types() {
    assert_eq!(emitted(&reply_api(), &[], &[]), "");
}

#[test]
fn an_excluded_field_gets_no_accessor_because_the_bridge_has_no_getter_for_it() {
    let out = emitted(&reply_api(), &["Content"], &["Message.content"]);
    assert!(!out.contains("contentValue"), "{out}");
    assert!(out.contains("summaryTextValue"), "{out}");
}

#[test]
fn a_type_without_a_text_field_gets_no_extension() {
    let mut api = reply_api();
    api.types[0].fields.retain(|field| field.name == "label");
    assert_eq!(emitted(&api, &["Content"], &[]), "");
}

fn api_with_extra_field(name: &str) -> ApiSurface {
    let mut api = reply_api();
    api.types[0].fields.push(FieldDef {
        name: name.into(),
        ty: TypeRef::String,
        ..Default::default()
    });
    api
}

#[test]
fn a_field_named_like_the_json_getter_fails_generation() {
    for name in ["content_json", "contentJson"] {
        let error = try_emit(&api_with_extra_field(name), &["Content"], &[]).expect_err(name);
        assert!(error.to_string().contains("`content_json`"), "{error}");
    }
}

#[test]
fn a_field_named_like_the_accessor_fails_generation() {
    for name in ["content_value", "contentValue"] {
        let error = try_emit(&api_with_extra_field(name), &["Content"], &[]).expect_err(name);
        assert!(error.to_string().contains("`contentValue()`"), "{error}");
    }
}

#[test]
fn a_colliding_field_that_the_bridge_excludes_does_not_fail_generation() {
    let api = api_with_extra_field("content_json");
    let out = try_emit(&api, &["Content"], &["Message.content_json"]).expect("excluded field has no getter");
    assert!(out.contains("contentValue"), "{out}");
}

#[test]
fn a_similar_name_that_is_not_a_collision_is_accepted() {
    let out = try_emit(&api_with_extra_field("content_values"), &["Content"], &[]).expect("no collision");
    assert!(out.contains("contentValue"), "{out}");
}
