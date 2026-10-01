use super::*;
use crate::core::ir::{DefaultValue, TypeDef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn defaulted_alt_text() -> FieldDef {
    let mut field = field("alt_text", TypeRef::Named("CaptionAltTextMode".to_string()));
    field.default = Some("/* serde(default) */".to_string());
    field.typed_default = Some(DefaultValue::EnumVariant("Preserve".to_string()));
    field
}

fn rendered_params(fields: Vec<FieldDef>) -> String {
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        fields,
        ..Default::default()
    };
    let enum_names: AHashSet<String> = ["CaptionAltTextMode".to_string()].into_iter().collect();
    gen_struct_constructor_stub_params(&typ, &enum_names, &AHashSet::new(), &AHashSet::new(), true).join("\n")
}

#[test]
fn non_suffix_bare_serde_default_stays_required_and_in_place() {
    let rendered = rendered_params(vec![
        field("llm", TypeRef::String),
        defaulted_alt_text(),
        field("prompt", TypeRef::String),
    ]);

    assert!(rendered.contains("string $altText"), "{rendered}");
    assert!(!rendered.contains("?string $altText"), "{rendered}");
    assert!(!rendered.contains("$altText = null"), "{rendered}");
    assert!(
        rendered.find("$llm").unwrap() < rendered.find("$altText").unwrap()
            && rendered.find("$altText").unwrap() < rendered.find("$prompt").unwrap(),
        "a non-suffix serde default must preserve declaration order: {rendered}"
    );
}

#[test]
fn suffix_bare_serde_default_is_nullable() {
    let rendered = rendered_params(vec![field("llm", TypeRef::String), defaulted_alt_text()]);

    assert!(rendered.contains("?string $altText = null"), "{rendered}");
}
