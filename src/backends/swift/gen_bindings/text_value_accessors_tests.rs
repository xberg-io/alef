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
    let mut out = String::new();
    emit(
        api,
        &text_types.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
        &HashSet::new(),
        &exclude_fields.iter().map(|f| f.to_string()).collect(),
        &HashSet::new(),
        &mut out,
    );
    out
}

#[test]
fn each_text_field_gets_an_accessor_that_decodes_its_json_getter() {
    let out = emitted(&reply_api(), &["Content"], &[]);
    assert!(out.starts_with("extension RustBridge.MessageRef {"), "{out}");
    assert!(out.contains("public func contentValue() throws -> Content? {"), "{out}");
    assert!(
        out.contains("try contentJson().map { try JSONDecoder().decode(Content.self, from: Data($0.toString().utf8)) }"),
        "{out}"
    );
    assert!(out.contains("public func summaryTextValue() throws -> Content {"), "{out}");
    assert!(
        out.contains("try JSONDecoder().decode(Content.self, from: Data(summaryTextJson().toString().utf8))"),
        "{out}"
    );
    assert!(!out.contains("labelValue"), "a field outside the text types gets no accessor:\n{out}");
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
