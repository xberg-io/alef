use super::enum_conversions::{emit_from_impl_for_enum, emit_from_mirror_to_core_enum};
use super::mirror::emit_mirror_enum;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};

/// A data variant field that is `Option<T>` in Rust must stay `Option<T>` in the mirror, and
/// both conversions must map through the option, so `Some("")` is not read back as `None`.
#[test]
fn optional_data_variant_field_stays_optional_through_the_mirror() {
    use crate::core::ir::FieldDef;
    let field = |name: &str, optional: bool| FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional,
        ..Default::default()
    };
    let en = EnumDef {
        name: "Event".to_string(),
        rust_path: "mylib::Event".to_string(),
        variants: vec![EnumVariant {
            name: "Message".to_string(),
            fields: vec![
                field("text", false),
                field("tag", true),
                FieldDef {
                    name: "count".to_string(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                    optional: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut mirror = String::new();
    emit_mirror_enum(&mut mirror, &en, "mylib", None);
    assert!(mirror.contains("tag: Option<String>,"), "got:\n{mirror}");
    assert!(mirror.contains("text: String,"), "got:\n{mirror}");

    let mut from_core = String::new();
    emit_from_impl_for_enum(&mut from_core, &en, "mylib", None);
    assert!(from_core.contains("count.map(|value| value as _)"), "got:\n{from_core}");
    assert!(!from_core.contains("unwrap_or_default"), "got:\n{from_core}");

    let mut to_core = String::new();
    emit_from_mirror_to_core_enum(&mut to_core, &en, "mylib", None);
    assert!(!to_core.contains("is_empty()"), "got:\n{to_core}");
    assert!(to_core.contains("count.map(|value| value as _)"), "got:\n{to_core}");
}
