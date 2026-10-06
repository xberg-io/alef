use super::*;
use crate::core::ir::FieldDef;

fn render_tagged(en: &EnumDef) -> String {
    let mut out = String::new();
    emit_serde_tagged_codable(en, &mut out, &SwiftMapper);
    out
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn newtype_variant(name: &str, ty: TypeRef) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        is_tuple: true,
        fields: vec![field("0", ty)],
        ..EnumVariant::default()
    }
}

fn struct_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        ..EnumVariant::default()
    }
}

fn tagged_enum(tag: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "FormatMetadata".to_string(),
        has_serde: true,
        serde_tag: Some(tag.to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        variants,
        ..EnumDef::default()
    }
}

#[test]
fn should_decode_an_internally_tagged_newtype_payload_from_the_same_decoder() {
    let en = tagged_enum(
        "format_type",
        vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
    );
    let out = render_tagged(&en);
    assert!(
        out.contains("self = .excel(field0: try ExcelMetadata(from: decoder))"),
        "{out}"
    );
}

#[test]
fn should_encode_an_internally_tagged_newtype_payload_into_the_tag_container() {
    let en = tagged_enum(
        "format_type",
        vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
    );
    let out = render_tagged(&en);
    assert!(out.contains("case .excel(let field0):"), "{out}");
    assert!(
        out.contains("try container.encode(\"excel\", forKey: .formatType)"),
        "{out}"
    );
    assert!(out.contains("try field0.encode(to: encoder)"), "{out}");
}

#[test]
fn should_not_give_an_internally_tagged_newtype_payload_a_positional_coding_key() {
    let en = tagged_enum(
        "format_type",
        vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
    );
    let out = render_tagged(&en);
    assert!(
        !out.contains("field0 = \"0\""),
        "serde flattens the payload; there is no \"0\" key on the wire: {out}"
    );
    assert!(
        !out.contains("forKey: .field0"),
        "serde flattens the payload; there is no \"0\" key on the wire: {out}"
    );
}

#[test]
fn should_keep_the_keyed_form_for_an_internally_tagged_struct_variant() {
    let en = tagged_enum(
        "type",
        vec![struct_variant(
            "Basic",
            vec![field("username", TypeRef::String), field("password", TypeRef::String)],
        )],
    );
    let out = render_tagged(&en);
    // No `= "wire"` alias when the Swift label already equals the wire name -- that is the
    // shape `AuthConfig` ships with today, and the alias is only emitted when they differ. ~keep
    assert!(out.contains("case username"), "{out}");
    assert!(out.contains("case password"), "{out}");
    assert!(!out.contains("case username = "), "{out}");
    assert!(
        out.contains(
            "self = .basic(username: try container.decode(String.self, forKey: .username), \
                 password: try container.decode(String.self, forKey: .password))"
        ),
        "{out}"
    );
    assert!(
        out.contains("try container.encode(username, forKey: .username)"),
        "{out}"
    );
    assert!(
        !out.contains("(from: decoder)"),
        "a named-field variant is not a flattened newtype: {out}"
    );
}

#[test]
fn should_keep_the_keyed_form_for_an_optional_newtype_payload() {
    let en = tagged_enum(
        "format_type",
        vec![newtype_variant(
            "Excel",
            TypeRef::Optional(Box::new(TypeRef::Named("ExcelMetadata".to_string()))),
        )],
    );
    let out = render_tagged(&en);
    assert!(
        out.contains("field0: try container.decodeIfPresent("),
        "`T?(from:)` is not expressible in Swift, so the keyed path is kept: {out}"
    );
    assert!(out.contains("forKey: .field0"), "{out}");
    assert!(out.contains("case field0 = \"0\""), "{out}");
    assert!(!out.contains("(from: decoder)"), "{out}");
}

#[test]
fn should_alias_a_snake_case_tag_key_onto_a_camel_case_swift_identifier() {
    let en = tagged_enum("format_type", vec![struct_variant("Excel", vec![])]);
    let out = render_tagged(&en);
    assert!(out.contains("case formatType = \"format_type\""), "{out}");
    assert!(out.contains("forKey: .formatType"), "{out}");
    assert!(!out.contains("case format_type"), "{out}");
    assert!(!out.contains("forKey: .format_type"), "{out}");
}

#[test]
fn should_alias_a_hyphenated_tag_key_onto_a_legal_swift_identifier() {
    let en = tagged_enum("format-type", vec![struct_variant("Excel", vec![])]);
    let out = render_tagged(&en);
    assert!(out.contains("case formatType = \"format-type\""), "{out}");
    assert!(out.contains("forKey: .formatType"), "{out}");
    assert!(!out.contains("case format-type"), "{out}");
}

#[test]
fn should_backtick_escape_a_tag_key_that_collides_with_a_swift_keyword() {
    let en = tagged_enum("protocol", vec![struct_variant("Excel", vec![])]);
    let out = render_tagged(&en);
    assert!(out.contains("case `protocol` = \"protocol\""), "{out}");
    assert!(out.contains("forKey: .`protocol`"), "{out}");
}

#[test]
fn should_give_the_conventional_type_tag_key_an_explicit_raw_value() {
    let en = tagged_enum("type", vec![struct_variant("Excel", vec![])]);
    let out = render_tagged(&en);
    assert!(out.contains("case type = \"type\""), "{out}");
    assert!(out.contains("forKey: .type"), "{out}");
}
