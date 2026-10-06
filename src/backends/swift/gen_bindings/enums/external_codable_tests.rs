use super::*;
use crate::core::ir::FieldDef;
use std::collections::HashSet;

fn external_enum() -> EnumDef {
    EnumDef {
        name: "OutputFormat".to_string(),
        has_serde: true,
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Plain".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

fn render(en: &EnumDef) -> String {
    let mut out = String::new();
    emit_enum(
        en,
        &mut out,
        &SwiftMapper,
        &HashSet::new(),
        &[],
        &EnumDeclarationCfg::new("sample", &[]),
    );
    out
}

#[test]
fn externally_tagged_enum_declares_a_custom_codable_body() {
    let out = render(&external_enum());
    assert!(
        out.contains("public init(from decoder: Decoder) throws {"),
        "got:\n{out}"
    );
    assert!(out.contains("struct __AlefExternalTagKey: CodingKey"), "got:\n{out}");
}

#[test]
fn a_unit_variant_decodes_from_a_bare_string_not_a_keyed_object() {
    let out = render(&external_enum());
    // serde writes a fieldless variant as a bare string; the synthesized Codable this
    // replaces expected `{"plain":{}}` and threw typeMismatch on the real value.
    assert!(out.contains("try? single.decode(String.self)"), "got:\n{out}");
    assert!(out.contains("case \"plain\":"), "got:\n{out}");
    assert!(out.contains("try single.encode(\"plain\")"), "got:\n{out}");
}

#[test]
fn a_newtype_variant_decodes_the_payload_directly_under_its_tag_key() {
    let out = render(&external_enum());
    // `{"custom":"foo"}` -- the value under the tag IS the payload, with no `field0` wrapper.
    assert!(
        out.contains("self = .custom(field0: try container.decode(String.self, forKey: key))"),
        "got:\n{out}"
    );
    assert!(
        out.contains("try container.encode(value, forKey: __AlefExternalTagKey(\"custom\"))"),
        "got:\n{out}"
    );
    assert!(
        !out.contains("\"field0\""),
        "payload must not be wrapped in a field0 key:\n{out}"
    );
}
